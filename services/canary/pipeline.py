"""Canary deployment decision engine (#414).

Routes consumer queries between the live ("blue") and candidate ("green")
oracle contracts, compares per-asset output, and decides whether to roll back
or wait for human-approved promotion.

Design constraints:

* **Routing is per asset, never per query.** An asset's bucket is
  ``sha256(salt || asset) mod 10_000`` and the asset is served by green iff
  ``bucket < canary_bps``. The rule ignores the caller and the ledger, so an
  asset is answered by exactly one version for the whole canary epoch and
  therefore in every ledger — split-brain is impossible by construction.
* **Correctness divergence, not error rate.** Both versions read the same
  source submissions and aggregate with deterministic integer math, so the
  default tolerance is **0 bps**: any difference in price, decimals or
  source count is a behavioural change and triggers rollback.
* **Every asset is compared**, including those routed to blue, so a bug that
  only affects a minority of assets cannot hide outside the canary slice.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import asdict, dataclass, field
from typing import Dict, Iterable, List, Optional

BLUE = "blue"
GREEN = "green"
DEFAULT_TOLERANCE_BPS = 0


def route(asset: str, canary_bps: int, salt: str) -> str:
    """Returns the single version that serves ``asset`` for this epoch."""
    if not 0 <= canary_bps <= 10_000:
        raise ValueError("canary_bps must be in [0, 10000]")
    digest = hashlib.sha256(f"{salt}:{asset}".encode()).digest()
    bucket = int.from_bytes(digest[:8], "big") % 10_000
    return GREEN if bucket < canary_bps else BLUE


def routing_table(assets: Iterable[str], canary_bps: int, salt: str) -> Dict[str, str]:
    return {a: route(a, canary_bps, salt) for a in assets}


@dataclass
class Divergence:
    asset: str
    field: str
    blue: object
    green: object
    delta_bps: Optional[int] = None


def compare(
    blue: Dict[str, dict],
    green: Dict[str, dict],
    tolerance_bps: int = DEFAULT_TOLERANCE_BPS,
) -> List[Divergence]:
    """Compares per-asset snapshots ``{asset: {price, decimals, num_sources}}``."""
    out: List[Divergence] = []
    for asset in sorted(set(blue) | set(green)):
        b, g = blue.get(asset), green.get(asset)
        if b is None or g is None:
            out.append(Divergence(asset, "presence", b is not None, g is not None))
            continue
        for key in ("decimals", "num_sources"):
            if b.get(key) != g.get(key):
                out.append(Divergence(asset, key, b.get(key), g.get(key)))
        bp, gp = int(b["price"]), int(g["price"])
        if bp != gp:
            delta = abs(gp - bp) * 10_000 // max(abs(bp), 1)
            if delta > tolerance_bps or tolerance_bps == 0:
                out.append(Divergence(asset, "price", bp, gp, delta))
    return out


@dataclass
class DeploymentRecord:
    blue_contract: str
    green_contract: str
    canary_bps: int
    salt: str
    tolerance_bps: int
    routing: Dict[str, str]
    divergences: List[dict]
    decision: str
    approved_by: Optional[str] = None
    evidence_digest: str = field(default="")

    def seal(self) -> "DeploymentRecord":
        body = asdict(self)
        body.pop("evidence_digest")
        self.evidence_digest = hashlib.sha256(
            json.dumps(body, sort_keys=True).encode()
        ).hexdigest()
        return self


def decide(divergences: List[Divergence]) -> str:
    """``rollback`` on any divergence, otherwise promotion awaits a human."""
    return "rollback" if divergences else "await_approval"


def run(args: argparse.Namespace) -> int:
    with open(args.blue_snapshot) as f:
        blue = json.load(f)
    with open(args.green_snapshot) as f:
        green = json.load(f)
    divs = compare(blue, green, args.tolerance_bps)
    record = DeploymentRecord(
        blue_contract=args.blue_contract,
        green_contract=args.green_contract,
        canary_bps=args.canary_bps,
        salt=args.salt,
        tolerance_bps=args.tolerance_bps,
        routing=routing_table(sorted(set(blue) | set(green)), args.canary_bps, args.salt),
        divergences=[asdict(d) for d in divs],
        decision=decide(divs),
    ).seal()
    with open(args.out, "w") as f:
        json.dump(asdict(record), f, indent=2, sort_keys=True)
    print(f"decision={record.decision} divergences={len(divs)} digest={record.evidence_digest}")
    return 1 if record.decision == "rollback" else 0


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--blue-snapshot", required=True)
    p.add_argument("--green-snapshot", required=True)
    p.add_argument("--blue-contract", required=True)
    p.add_argument("--green-contract", required=True)
    p.add_argument("--canary-bps", type=int, default=1_000)
    p.add_argument("--salt", required=True)
    p.add_argument("--tolerance-bps", type=int, default=DEFAULT_TOLERANCE_BPS)
    p.add_argument("--out", required=True)
    return run(p.parse_args(argv))


if __name__ == "__main__":
    sys.exit(main())
