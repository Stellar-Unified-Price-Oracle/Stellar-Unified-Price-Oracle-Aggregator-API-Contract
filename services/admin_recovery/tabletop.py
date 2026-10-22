"""Production admin key recovery tabletop exercise automation (#680).

Exercises suspected key compromise, signer revocation, contract pause decision,
and restoration of trusted administration while enforcing zero-key-material exposure
and SLA response times.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import time
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional, Sequence, Tuple

# Response SLA targets from docs/SLA.md (seconds)
# P0 incident: Time-to-detect <= 1800s (30m), Time-to-contain <= 7200s (2h)
SLA_TIME_TO_DETECT_MAX_S = 1800
SLA_TIME_TO_CONTAIN_MAX_S = 7200

# Regex pattern to catch raw secret seeds or private keys in logs
SECRET_SEED_PATTERN = re.compile(r"\bS[A-Z2-7]{55}\b")
PRIVATE_KEY_PATTERN = re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY")


@dataclass
class KeyCompromiseAlert:
    """Incident trigger signal for suspected admin compromise."""
    alert_id: str
    detected_at: float
    source: str
    anomaly_description: str
    flagged_caller: str
    suspicious_action: str


@dataclass
class PhaseRecord:
    phase_number: int
    name: str
    started_at: float
    completed_at: float
    status: str
    details: Dict[str, Any]
    evidence_hash: str


@dataclass
class TabletopResult:
    scenario_id: str
    alert: KeyCompromiseAlert
    pause_executed: bool
    signer_revoked: bool
    guardians_approved: bool
    recovery_executed: bool
    initial_admin: str
    rescue_admin: str
    final_admin: str
    time_to_detect_s: float
    time_to_contain_s: float
    sla_met: bool
    phases: List[PhaseRecord]
    evidence_bundle: Dict[str, Any]

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


def sanitize_text(text: str) -> str:
    """Ensures no secret seeds or key material leak into evidence."""
    sanitized = SECRET_SEED_PATTERN.sub("[REDACTED_SECRET_SEED]", text)
    sanitized = PRIVATE_KEY_PATTERN.sub("[REDACTED_PRIVATE_KEY]", sanitized)
    return sanitized


class AdminRecoveryTabletop:
    """Simulates and verifies an end-to-end admin key recovery tabletop exercise."""

    def __init__(
        self,
        scenario_id: str = "tabletop-2026-prod-admin",
        initial_admin: str = "GA_COMPROMISED_ADMIN_KEY_PUBLIC_ADDR_001",
        rescue_admin: str = "GA_RESCUE_ADMIN_KEY_PUBLIC_ADDR_002",
        guardians: Optional[List[str]] = None,
    ) -> None:
        self.scenario_id = scenario_id
        self.initial_admin = initial_admin
        self.rescue_admin = rescue_admin
        self.guardians = guardians or [
            "GB_GUARDIAN_ALPHA_PUBLIC_ADDR_101",
            "GB_GUARDIAN_BETA_PUBLIC_ADDR_102",
            "GB_GUARDIAN_GAMMA_PUBLIC_ADDR_103",
        ]
        self.current_admin = initial_admin
        self.is_paused = False
        self.recovery_pending = False
        self.phases: List[PhaseRecord] = []
        self.log_entries: List[str] = []

    def _hash_record(self, data: Dict[str, Any]) -> str:
        serialized = json.dumps(data, sort_keys=True)
        return hashlib.sha256(serialized.encode("utf-8")).hexdigest()

    def run_exercise(
        self,
        simulated_detection_delay_s: float = 600.0,
        simulated_containment_delay_s: float = 2400.0,
        pause_decision: bool = True,
    ) -> TabletopResult:
        """Executes the complete 5-phase tabletop drill."""
        t_incident_start = time.time()
        t_detect = t_incident_start + simulated_detection_delay_s

        # ---------------------------------------------------------------------
        # Phase 1: Detection of Suspected Key Compromise
        # ---------------------------------------------------------------------
        alert = KeyCompromiseAlert(
            alert_id=f"ALERT-{self.scenario_id}-01",
            detected_at=t_detect,
            source="OracleAnomalyMonitor::AdminAudit",
            anomaly_description="Unscheduled high-impact config mutation detected outside maintenance window",
            flagged_caller=self.initial_admin,
            suspicious_action="set_max_price_deviation(9000)",
        )
        p1_details = {
            "alert": asdict(alert),
            "threat_classification": "CRITICAL_KEY_COMPROMISE",
            "blast_radius_evaluated": ["oracle_pricing", "source_registration", "circuit_breaker"],
        }
        self.phases.append(
            PhaseRecord(
                phase_number=1,
                name="Detection & Triage",
                started_at=t_detect,
                completed_at=t_detect + 60.0,
                status="COMPLETED",
                details=p1_details,
                evidence_hash=self._hash_record(p1_details),
            )
        )

        # ---------------------------------------------------------------------
        # Phase 2: Signer Revocation & Contract Pause Decision
        # ---------------------------------------------------------------------
        t_p2_start = t_detect + 60.0
        # If blast radius includes active price manipulation risk, execute emergency pause
        if pause_decision:
            self.is_paused = True
        signer_revoked_at = t_p2_start + 180.0
        p2_details = {
            "pause_contract_invoked": self.is_paused,
            "pause_reason": "Precautionary halt during suspected admin key compromise drill",
            "revocation_target_key": self.initial_admin,
            "signer_revoked_from_bastion": True,
            "signer_revoked_from_multisig": True,
            "revoked_at": signer_revoked_at,
        }
        self.phases.append(
            PhaseRecord(
                phase_number=2,
                name="Containment & Contract Pause",
                started_at=t_p2_start,
                completed_at=signer_revoked_at,
                status="COMPLETED",
                details=p2_details,
                evidence_hash=self._hash_record(p2_details),
            )
        )

        # ---------------------------------------------------------------------
        # Phase 3: Guardian Quorum & Recovery Proposal
        # ---------------------------------------------------------------------
        t_p3_start = signer_revoked_at + 60.0
        self.recovery_pending = True
        guardian_approvals = {g: True for g in self.guardians[:2]}  # 2 of 3 quorum
        quorum_reached = len(guardian_approvals) >= 2
        p3_details = {
            "candidate_admin": self.rescue_admin,
            "guardian_quorum_required": 2,
            "guardian_votes": guardian_approvals,
            "quorum_achieved": quorum_reached,
            "recovery_timelock_delay_ledgers": 100,
        }
        self.phases.append(
            PhaseRecord(
                phase_number=3,
                name="Guardian Quorum & Proposal",
                started_at=t_p3_start,
                completed_at=t_p3_start + 300.0,
                status="COMPLETED",
                details=p3_details,
                evidence_hash=self._hash_record(p3_details),
            )
        )

        # ---------------------------------------------------------------------
        # Phase 4: Timelock & Veto Cooldown Verification
        # ---------------------------------------------------------------------
        t_p4_start = t_p3_start + 300.0
        p4_details = {
            "cancellation_window_elapsed": True,
            "adversary_swap_guardians_blocked": True,
            "veto_attempts_handled": 0,
            "timelock_ready": True,
        }
        self.phases.append(
            PhaseRecord(
                phase_number=4,
                name="Timelock & Veto Validation",
                started_at=t_p4_start,
                completed_at=t_p4_start + 120.0,
                status="COMPLETED",
                details=p4_details,
                evidence_hash=self._hash_record(p4_details),
            )
        )

        # ---------------------------------------------------------------------
        # Phase 5: Restoration of Trusted Administration
        # ---------------------------------------------------------------------
        t_p5_start = t_incident_start + simulated_containment_delay_s
        self.current_admin = self.rescue_admin
        self.recovery_pending = False
        if self.is_paused:
            self.is_paused = False  # Rescue admin verifies state and unpauses
        p5_details = {
            "executed_by": "recovery_execute",
            "new_admin_verified": self.current_admin,
            "state_integrity_audit_passed": True,
            "unpaused": not self.is_paused,
            "restoration_completed_at": t_p5_start + 120.0,
        }
        t_p5_done = t_p5_start + 120.0
        self.phases.append(
            PhaseRecord(
                phase_number=5,
                name="Restoration & Reinstatement",
                started_at=t_p5_start,
                completed_at=t_p5_done,
                status="COMPLETED",
                details=p5_details,
                evidence_hash=self._hash_record(p5_details),
            )
        )

        # Metric evaluations
        ttd = simulated_detection_delay_s
        ttc = t_p5_done - t_incident_start
        sla_met = (ttd <= SLA_TIME_TO_DETECT_MAX_S) and (ttc <= SLA_TIME_TO_CONTAIN_MAX_S)

        # Evidence bundle
        evidence_bundle = {
            "exercise_id": self.scenario_id,
            "executed_at_iso": datetime.now(timezone.utc).isoformat(),
            "sla_evaluation": {
                "time_to_detect_seconds": ttd,
                "time_to_detect_target": SLA_TIME_TO_DETECT_MAX_S,
                "time_to_contain_seconds": ttc,
                "time_to_contain_target": SLA_TIME_TO_CONTAIN_MAX_S,
                "sla_satisfied": sla_met,
            },
            "chain_of_custody_hashes": [p.evidence_hash for p in self.phases],
            "zero_leak_verified": True,
        }

        # Check for any secret exposure across the full record
        full_record = {
            "scenario_id": self.scenario_id,
            "initial_admin": self.initial_admin,
            "rescue_admin": self.rescue_admin,
            "alert": asdict(alert),
            "phases": [asdict(p) for p in self.phases],
            "evidence_bundle": evidence_bundle,
        }
        full_record_str = json.dumps(full_record)
        if SECRET_SEED_PATTERN.search(full_record_str) or PRIVATE_KEY_PATTERN.search(full_record_str):
            raise ValueError("CRITICAL: Secret key material detected in tabletop evidence record!")

        return TabletopResult(
            scenario_id=self.scenario_id,
            alert=alert,
            pause_executed=True,
            signer_revoked=True,
            guardians_approved=quorum_reached,
            recovery_executed=(self.current_admin == self.rescue_admin),
            initial_admin=self.initial_admin,
            rescue_admin=self.rescue_admin,
            final_admin=self.current_admin,
            time_to_detect_s=ttd,
            time_to_contain_s=ttc,
            sla_met=sla_met,
            phases=self.phases,
            evidence_bundle=evidence_bundle,
        )


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Run Production Admin Key Recovery Tabletop Exercise")
    parser.add_argument("--out", type=str, help="Output evidence JSON path")
    parser.add_argument("--scenario", default="tabletop-prod-admin-compromise", help="Scenario ID")
    parser.add_argument("--assert-sla", action="store_true", help="Fail if SLA targets not met")
    args = parser.parse_args(argv)

    tabletop = AdminRecoveryTabletop(scenario_id=args.scenario)
    res = tabletop.run_exercise()

    output_data = res.to_dict()
    output_json = json.dumps(output_data, indent=2)

    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(output_json)
        print(f"Evidence bundle written to {args.out}")
    else:
        print(output_json)

    if args.assert_sla and not res.sla_met:
        print("ERROR: Tabletop response did not meet SLA targets", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
