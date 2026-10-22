"""Asset metadata provenance and change history (#705).

Tracks the full lifecycle and audit trail of asset metadata configurations:
- decimals, ticker, asset type
- source mappings and weights
- staleness threshold (seconds)
- max deviation threshold (basis points)

Every metadata revision is cryptographically bound into a per-asset hash chain
linking predecessor hashes, proposer identity, approving reviewer, evidence/rationale,
and effective ledger/timestamp. Provides point-in-time historical reconstruction
and automated tamper detection.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

# Domain separator for asset metadata provenance commitments to prevent cross-protocol collisions
DOMAIN_SEPARATOR = "ASSET_META_V1"
GENESIS_PREVIOUS_HASH = "0" * 64


@dataclass(frozen=True)
class AssetMetadataConfig:
    """The configurable operational parameters of an oracle asset."""

    decimals: int
    ticker: str
    asset_type: str  # e.g., "native", "credit_alphanum4", "credit_alphanum12"
    sources: List[str]  # Authorized price feed source accounts
    source_weights_bps: List[int]  # Corresponding weights in basis points (sum to 10,000)
    staleness_threshold_seconds: int = 300
    deviation_threshold_bps: int = 500  # 5% max deviation

    def validate(self) -> None:
        if self.decimals < 0 or self.decimals > 18:
            raise ValueError(f"Decimals must be between 0 and 18, got {self.decimals}")
        if not self.ticker or len(self.ticker) > 12:
            raise ValueError(f"Ticker must be non-empty and <= 12 characters, got '{self.ticker}'")
        if not self.sources:
            raise ValueError("Asset must configure at least one price source")
        if len(self.sources) != len(self.source_weights_bps):
            raise ValueError("Sources count must match source weights count")
        if sum(self.source_weights_bps) != 10_000:
            raise ValueError(f"Source weights must sum to 10,000 bps, got {sum(self.source_weights_bps)}")
        if self.staleness_threshold_seconds <= 0:
            raise ValueError("Staleness threshold must be positive")
        if self.deviation_threshold_bps <= 0:
            raise ValueError("Deviation threshold must be positive")

    def to_dict(self) -> Dict[str, Any]:
        return {
            "decimals": self.decimals,
            "ticker": self.ticker,
            "asset_type": self.asset_type,
            "sources": list(self.sources),
            "source_weights_bps": list(self.source_weights_bps),
            "staleness_threshold_seconds": self.staleness_threshold_seconds,
            "deviation_threshold_bps": self.deviation_threshold_bps,
        }


@dataclass(frozen=True)
class AssetMetadataRevision:
    """An immutable, tamper-evident record of an asset metadata revision."""

    asset_id: str
    revision_number: int
    config: AssetMetadataConfig
    prior_config: Optional[AssetMetadataConfig]
    effective_ledger: int
    effective_timestamp: int
    proposer: str
    reviewer: str
    evidence_url: str
    rationale: str
    previous_hash: str
    hash: str

    def compute_diff(self) -> Dict[str, Tuple[Any, Any]]:
        """Return dict of field_name -> (prior_value, new_value) for modified fields."""
        if self.prior_config is None:
            return {"*": (None, self.config.to_dict())}
        diff: Dict[str, Tuple[Any, Any]] = {}
        curr = self.config.to_dict()
        prior = self.prior_config.to_dict()
        for k, v in curr.items():
            if prior.get(k) != v:
                diff[k] = (prior.get(k), v)
        return diff

    def to_dict(self) -> Dict[str, Any]:
        return {
            "asset_id": self.asset_id,
            "revision_number": self.revision_number,
            "config": self.config.to_dict(),
            "prior_config": self.prior_config.to_dict() if self.prior_config else None,
            "effective_ledger": self.effective_ledger,
            "effective_timestamp": self.effective_timestamp,
            "proposer": self.proposer,
            "reviewer": self.reviewer,
            "evidence_url": self.evidence_url,
            "rationale": self.rationale,
            "previous_hash": self.previous_hash,
            "hash": self.hash,
        }


def compute_revision_hash(
    asset_id: str,
    revision_number: int,
    config: AssetMetadataConfig,
    prior_config: Optional[AssetMetadataConfig],
    effective_ledger: int,
    effective_timestamp: int,
    proposer: str,
    reviewer: str,
    evidence_url: str,
    rationale: str,
    previous_hash: str,
) -> str:
    """Compute deterministic SHA-256 hash commitment over revision payload."""
    payload = {
        "domain": DOMAIN_SEPARATOR,
        "asset_id": asset_id,
        "revision_number": revision_number,
        "config": config.to_dict(),
        "prior_config": prior_config.to_dict() if prior_config else None,
        "effective_ledger": effective_ledger,
        "effective_timestamp": effective_timestamp,
        "proposer": proposer,
        "reviewer": reviewer,
        "evidence_url": evidence_url,
        "rationale": rationale,
        "previous_hash": previous_hash,
    }
    canonical_bytes = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(canonical_bytes).hexdigest()


class AssetMetadataProvenanceTracker:
    """In-memory or persistent store managing asset metadata provenance chains."""

    def __init__(self) -> None:
        # asset_id -> list of revisions in ascending order
        self._chains: Dict[str, List[AssetMetadataRevision]] = {}

    def get_latest_revision(self, asset_id: str) -> Optional[AssetMetadataRevision]:
        chain = self._chains.get(asset_id, [])
        return chain[-1] if chain else None

    def propose_and_commit_revision(
        self,
        asset_id: str,
        config: AssetMetadataConfig,
        effective_ledger: int,
        effective_timestamp: int,
        proposer: str,
        reviewer: str,
        evidence_url: str,
        rationale: str,
    ) -> AssetMetadataRevision:
        """Create, cryptographically hash, and commit a new metadata revision."""
        config.validate()
        chain = self._chains.setdefault(asset_id, [])

        if not chain:
            revision_number = 1
            previous_hash = GENESIS_PREVIOUS_HASH
            prior_config = None
        else:
            latest = chain[-1]
            if effective_ledger <= latest.effective_ledger:
                raise ValueError(
                    f"Effective ledger {effective_ledger} must be strictly greater than latest ledger {latest.effective_ledger}"
                )
            if effective_timestamp < latest.effective_timestamp:
                raise ValueError(
                    f"Effective timestamp {effective_timestamp} cannot be earlier than latest timestamp {latest.effective_timestamp}"
                )
            revision_number = latest.revision_number + 1
            previous_hash = latest.hash
            prior_config = latest.config

        rev_hash = compute_revision_hash(
            asset_id=asset_id,
            revision_number=revision_number,
            config=config,
            prior_config=prior_config,
            effective_ledger=effective_ledger,
            effective_timestamp=effective_timestamp,
            proposer=proposer,
            reviewer=reviewer,
            evidence_url=evidence_url,
            rationale=rationale,
            previous_hash=previous_hash,
        )

        revision = AssetMetadataRevision(
            asset_id=asset_id,
            revision_number=revision_number,
            config=config,
            prior_config=prior_config,
            effective_ledger=effective_ledger,
            effective_timestamp=effective_timestamp,
            proposer=proposer,
            reviewer=reviewer,
            evidence_url=evidence_url,
            rationale=rationale,
            previous_hash=previous_hash,
            hash=rev_hash,
        )

        chain.append(revision)
        return revision

    def get_history(self, asset_id: str) -> List[AssetMetadataRevision]:
        return list(self._chains.get(asset_id, []))

    def get_metadata_at_ledger(self, asset_id: str, ledger: int) -> Optional[AssetMetadataConfig]:
        """Historical point-in-time lookup for asset metadata at a given ledger."""
        chain = self._chains.get(asset_id, [])
        active: Optional[AssetMetadataConfig] = None
        for rev in chain:
            if rev.effective_ledger <= ledger:
                active = rev.config
            else:
                break
        return active

    def get_metadata_at_timestamp(self, asset_id: str, timestamp: int) -> Optional[AssetMetadataConfig]:
        """Historical point-in-time lookup for asset metadata at a given timestamp."""
        chain = self._chains.get(asset_id, [])
        active: Optional[AssetMetadataConfig] = None
        for rev in chain:
            if rev.effective_timestamp <= timestamp:
                active = rev.config
            else:
                break
        return active

    def verify_provenance_chain(self, asset_id: str) -> Tuple[bool, List[str]]:
        """Verify internal consistency and cryptographic hash linkage of an asset's revisions."""
        chain = self._chains.get(asset_id, [])
        if not chain:
            return True, []

        errors: List[str] = []
        expected_prev_hash = GENESIS_PREVIOUS_HASH

        for idx, rev in enumerate(chain):
            # 1. Chain linkage
            if rev.previous_hash != expected_prev_hash:
                errors.append(
                    f"Revision {rev.revision_number}: previous_hash mismatch! Expected {expected_prev_hash}, got {rev.previous_hash}"
                )

            # 2. Recompute internal commitment
            recomputed = compute_revision_hash(
                asset_id=rev.asset_id,
                revision_number=rev.revision_number,
                config=rev.config,
                prior_config=rev.prior_config,
                effective_ledger=rev.effective_ledger,
                effective_timestamp=rev.effective_timestamp,
                proposer=rev.proposer,
                reviewer=rev.reviewer,
                evidence_url=rev.evidence_url,
                rationale=rev.rationale,
                previous_hash=rev.previous_hash,
            )
            if recomputed != rev.hash:
                errors.append(
                    f"Revision {rev.revision_number}: hash commitment mismatch! Expected {recomputed}, recorded {rev.hash}"
                )

            # 3. Revision numbering and monotonic constraints
            if rev.revision_number != idx + 1:
                errors.append(f"Revision index discontinuity: expected {idx + 1}, got {rev.revision_number}")

            expected_prev_hash = rev.hash

        return len(errors) == 0, errors

    def export_audit_markdown(self, asset_id: str) -> str:
        """Export comprehensive audit trail formatted as Markdown."""
        chain = self.get_history(asset_id)
        if not chain:
            return f"# Asset Metadata Provenance: {asset_id}\n\nNo revisions recorded for this asset.\n"

        lines = [
            f"# Asset Metadata Provenance & Audit Trail: `{asset_id}`",
            "",
            f"**Total Revisions:** {len(chain)}  ",
            f"**Latest Revision:** #{chain[-1].revision_number} (Commitment: `{chain[-1].hash[:16]}...`)  ",
            "",
            "## Revision History Table",
            "",
            "| Rev # | Effective Ledger | Effective Date (UTC) | Ticker | Decimals | Proposer | Reviewer | Commitment Hash |",
            "|---|---|---|---|---|---|---|---|",
        ]

        for r in chain:
            dt_str = datetime.fromtimestamp(r.effective_timestamp, timezone.utc).strftime("%Y-%m-%d %H:%M")
            lines.append(
                f"| #{r.revision_number} | {r.effective_ledger} | {dt_str} | {r.config.ticker} | {r.config.decimals} | `{r.proposer}` | `{r.reviewer}` | `{r.hash[:12]}...` |"
            )

        lines.extend([
            "",
            "## Detailed Revision Log",
            "",
        ])

        for r in chain:
            lines.append(f"### Revision #{r.revision_number} (Effective Ledger: {r.effective_ledger})")
            lines.append(f"* **Hash Commitment:** `{r.hash}`")
            lines.append(f"* **Previous Hash:** `{r.previous_hash}`")
            lines.append(f"* **Proposer:** `{r.proposer}` | **Reviewer:** `{r.reviewer}`")
            lines.append(f"* **Evidence URL:** [{r.evidence_url}]({r.evidence_url})")
            lines.append(f"* **Rationale:** {r.rationale}")
            lines.append("* **Configuration:**")
            lines.append(f"  * Ticker: `{r.config.ticker}` | Decimals: `{r.config.decimals}` | Type: `{r.config.asset_type}`")
            lines.append(f"  * Sources ({len(r.config.sources)}): `{', '.join(r.config.sources)}`")
            lines.append(f"  * Weights (bps): `{r.config.source_weights_bps}`")
            lines.append(f"  * Staleness Threshold: {r.config.staleness_threshold_seconds}s | Max Deviation: {r.config.deviation_threshold_bps} bps")

            diff = r.compute_diff()
            if r.revision_number > 1 and diff:
                lines.append("* **Changes from Prior Revision:**")
                for field_name, (old_v, new_v) in diff.items():
                    lines.append(f"  * `{field_name}`: `{old_v}` $\\rightarrow$ `{new_v}`")
            lines.append("")

        return "\n".join(lines)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Asset metadata provenance and change history (#705)")
    parser.add_argument("--asset", default="XLM", help="Asset identifier")
    parser.add_argument("--action", choices=["demo", "verify"], default="demo", help="Action to execute")
    parser.add_argument("--json", action="store_true", help="Output audit log as JSON")
    parser.add_argument("--report", help="Save markdown audit report to file")

    args = parser.parse_args(argv)

    tracker = AssetMetadataProvenanceTracker()

    # Create initial genesis revision
    cfg1 = AssetMetadataConfig(
        decimals=7,
        ticker="XLM",
        asset_type="native",
        sources=["G_BINANCE", "G_COINBASE", "G_KRAKEN"],
        source_weights_bps=[4000, 3000, 3000],
        staleness_threshold_seconds=300,
        deviation_threshold_bps=500,
    )
    tracker.propose_and_commit_revision(
        asset_id="XLM",
        config=cfg1,
        effective_ledger=1000,
        effective_timestamp=1700000000,
        proposer="operator:governance-admin",
        reviewer="approver:multisig-signers",
        evidence_url="https://governance.stellar.org/proposals/SEP-0040-XLM",
        rationale="Initial asset registration and weighting configuration.",
    )

    # Revision 2: Source re-weighting
    cfg2 = AssetMetadataConfig(
        decimals=7,
        ticker="XLM",
        asset_type="native",
        sources=["G_BINANCE", "G_COINBASE", "G_KRAKEN", "G_BITSTAMP"],
        source_weights_bps=[2500, 2500, 2500, 2500],
        staleness_threshold_seconds=300,
        deviation_threshold_bps=400,
    )
    tracker.propose_and_commit_revision(
        asset_id="XLM",
        config=cfg2,
        effective_ledger=5000,
        effective_timestamp=1700020000,
        proposer="source-onboarding",
        reviewer="risk-committee",
        evidence_url="https://governance.stellar.org/proposals/SEP-0040-XLM-REWEIGHT",
        rationale="Onboard Bitstamp source and distribute equal 2500 bps weights.",
    )

    valid, errors = tracker.verify_provenance_chain("XLM")
    if not valid:
        print(f"ERROR: Provenance verification failed: {errors}", file=sys.stderr)
        return 1

    if args.report:
        with open(args.report, "w", encoding="utf-8") as f:
            f.write(tracker.export_audit_markdown("XLM"))

    if args.json:
        data = [r.to_dict() for r in tracker.get_history("XLM")]
        print(json.dumps(data, indent=2))
    else:
        print(tracker.export_audit_markdown("XLM"))

    return 0


if __name__ == "__main__":
    sys.exit(main())
