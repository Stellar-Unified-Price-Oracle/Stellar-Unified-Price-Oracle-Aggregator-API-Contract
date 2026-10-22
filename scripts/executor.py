#!/usr/bin/env python3
"""
Stellar Price Oracle — Off-chain Executor Service
==================================================
Monitors pending operations on the PriceOracle contract and auto-executes
them once the configurable expiry delay has elapsed.

Acceptance criteria implemented
--------------------------------
✓  Monitors via contract events (ledger-based polling with event cursor tracking)
✓  Auto-executes when delay elapses (configurable grace ledgers after creation)
✓  Error handling and retry (exponential back-off, configurable max attempts)
✓  Alerting on execution failures (webhook + stdout alerting, extensible)

Usage
-----
    python scripts/executor.py [--config executor_config.json]

Configuration keys (all override-able via environment variables)
-----------------------------------------------------------------
    ORACLE_CONTRACT_ID   — Soroban contract address
    STELLAR_NETWORK      — "testnet" | "mainnet" | custom RPC URL
    ADMIN_SECRET_KEY     — Admin account secret key (S…)
    POLL_INTERVAL_SECS   — Seconds between ledger polls (default: 6)
    GRACE_LEDGERS        — Ledgers after creation before auto-exec (default: 0)
    MAX_RETRY_ATTEMPTS   — Per-operation retry cap (default: 5)
    ALERT_WEBHOOK_URL    — Optional HTTP POST endpoint for failure alerts
    LOG_LEVEL            — Logging level: DEBUG | INFO | WARNING | ERROR

Dependencies
------------
    pip install stellar-sdk requests

"""

from __future__ import annotations

import json
import logging
import os
import sys
import time
import traceback
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Dict, List, Optional

# ---------------------------------------------------------------------------
# Optional dependency guard — give a clear error if stellar-sdk is missing
# ---------------------------------------------------------------------------
try:
    from stellar_sdk import (
        Keypair,
        Network,
        Server,
        SorobanServer,
        TransactionBuilder,
        scval,
    )
    from stellar_sdk.soroban_rpc import GetEventsRequest, EventFilter
    from stellar_sdk.exceptions import SdkError
    import requests
except ImportError as exc:
    sys.exit(
        f"Missing dependency: {exc}\n"
        "Install with: pip install stellar-sdk requests"
    )

# ---------------------------------------------------------------------------
# Logging setup
# ---------------------------------------------------------------------------
_LOG_LEVEL = os.environ.get("LOG_LEVEL", "INFO").upper()
logging.basicConfig(
    level=getattr(logging, _LOG_LEVEL, logging.INFO),
    format="%(asctime)s [%(levelname)s] %(name)s — %(message)s",
    datefmt="%Y-%m-%dT%H:%M:%SZ",
)
log = logging.getLogger("oracle_executor")

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

NETWORK_PRESETS: Dict[str, Dict[str, str]] = {
    "testnet": {
        "horizon_url": "https://horizon-testnet.stellar.org",
        "soroban_rpc_url": "https://soroban-testnet.stellar.org",
        "passphrase": Network.TESTNET_NETWORK_PASSPHRASE,
    },
    "mainnet": {
        "horizon_url": "https://horizon.stellar.org",
        "soroban_rpc_url": "https://soroban.stellar.org",
        "passphrase": Network.PUBLIC_NETWORK_PASSPHRASE,
    },
}


@dataclass
class ExecutorConfig:
    contract_id: str
    admin_secret_key: str
    network: str = "testnet"
    poll_interval_secs: float = 6.0
    grace_ledgers: int = 0
    max_retry_attempts: int = 5
    alert_webhook_url: Optional[str] = None
    # Internal — tracks last-seen event ledger cursor for incremental polling
    _event_cursor: Optional[str] = field(default=None, repr=False)

    # ---- Derived properties ----

    @property
    def horizon_url(self) -> str:
        return NETWORK_PRESETS.get(self.network, {}).get(
            "horizon_url", self.network
        )

    @property
    def soroban_rpc_url(self) -> str:
        return NETWORK_PRESETS.get(self.network, {}).get(
            "soroban_rpc_url", self.network
        )

    @property
    def network_passphrase(self) -> str:
        return NETWORK_PRESETS.get(self.network, {}).get(
            "passphrase", Network.TESTNET_NETWORK_PASSPHRASE
        )

    # ---- Factory ----

    @classmethod
    def from_env_and_file(cls, config_path: Optional[str] = None) -> "ExecutorConfig":
        """Build config from optional JSON file, overridden by environment variables."""
        raw: Dict[str, Any] = {}
        if config_path and os.path.exists(config_path):
            with open(config_path, encoding="utf-8") as fh:
                raw = json.load(fh)
            log.info("Loaded config from %s", config_path)

        def _get(key: str, default: Any = None) -> Any:
            env_key = key.upper()
            return os.environ.get(env_key, raw.get(key.lower(), default))

        contract_id = _get("ORACLE_CONTRACT_ID")
        admin_secret_key = _get("ADMIN_SECRET_KEY")
        if not contract_id or not admin_secret_key:
            sys.exit(
                "ORACLE_CONTRACT_ID and ADMIN_SECRET_KEY must be set "
                "(via environment variable or config file)."
            )

        return cls(
            contract_id=contract_id,
            admin_secret_key=admin_secret_key,
            network=_get("STELLAR_NETWORK", "testnet"),
            poll_interval_secs=float(_get("POLL_INTERVAL_SECS", 6)),
            grace_ledgers=int(_get("GRACE_LEDGERS", 0)),
            max_retry_attempts=int(_get("MAX_RETRY_ATTEMPTS", 5)),
            alert_webhook_url=_get("ALERT_WEBHOOK_URL"),
        )


# ---------------------------------------------------------------------------
# Alerting
# ---------------------------------------------------------------------------

def send_alert(config: ExecutorConfig, subject: str, body: str) -> None:
    """Dispatch a failure alert.  Logs to stderr and optionally POSTs to a webhook."""
    log.error("ALERT — %s: %s", subject, body)
    if not config.alert_webhook_url:
        return
    payload = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "subject": subject,
        "body": body,
        "contract_id": config.contract_id,
        "network": config.network,
    }
    try:
        resp = requests.post(
            config.alert_webhook_url,
            json=payload,
            timeout=10,
            headers={"Content-Type": "application/json"},
        )
        resp.raise_for_status()
        log.debug("Alert webhook responded %s", resp.status_code)
    except requests.RequestException as exc:
        log.warning("Failed to deliver alert to webhook: %s", exc)


# ---------------------------------------------------------------------------
# Retry helper
# ---------------------------------------------------------------------------

def with_retry(
    fn,
    *args,
    max_attempts: int = 5,
    base_delay: float = 2.0,
    label: str = "operation",
    **kwargs,
) -> Any:
    """Call *fn* with exponential back-off.  Returns result or raises on exhaustion."""
    last_exc: Optional[Exception] = None
    for attempt in range(1, max_attempts + 1):
        try:
            return fn(*args, **kwargs)
        except Exception as exc:  # noqa: BLE001
            last_exc = exc
            delay = base_delay * (2 ** (attempt - 1))
            log.warning(
                "Attempt %d/%d for %s failed (%s). Retrying in %.1fs…",
                attempt,
                max_attempts,
                label,
                exc,
                delay,
            )
            if attempt < max_attempts:
                time.sleep(delay)
    raise RuntimeError(
        f"All {max_attempts} attempts for '{label}' failed. "
        f"Last error: {last_exc}"
    ) from last_exc


# ---------------------------------------------------------------------------
# Stellar / Soroban helpers
# ---------------------------------------------------------------------------

def build_soroban_server(config: ExecutorConfig) -> SorobanServer:
    return SorobanServer(server_url=config.soroban_rpc_url)


def get_current_ledger(server: SorobanServer) -> int:
    """Return the latest confirmed ledger sequence number."""
    resp = server.get_latest_ledger()
    return resp.sequence


def call_contract_get_ids(
    server: SorobanServer,
    config: ExecutorConfig,
    keypair: Keypair,
) -> List[int]:
    """
    Call get_pending_operation_ids() on the contract (read-only simulation).
    Returns a list of pending operation ids (u64 encoded as Python ints).
    """
    source_account = server.load_account(keypair.public_key)
    tx = (
        TransactionBuilder(
            source_account=source_account,
            network_passphrase=config.network_passphrase,
            base_fee=300,
        )
        .append_invoke_contract_function_op(
            contract_id=config.contract_id,
            function_name="get_pending_operation_ids",
            parameters=[],
        )
        .build()
    )
    sim = server.simulate_transaction(tx)
    if sim.error:
        raise RuntimeError(f"Simulation error: {sim.error}")
    if not sim.results:
        return []
    raw = sim.results[0].xdr
    vec = scval.from_vec(scval.from_xdr(raw))
    return [scval.from_uint64(item) for item in vec]


def call_contract_get_operation(
    server: SorobanServer,
    config: ExecutorConfig,
    keypair: Keypair,
    operation_id: int,
) -> Dict[str, Any]:
    """
    Call get_operation(id) on the contract (read-only simulation).
    Returns a dict with keys: id, kind, args, created_at_ledger,
    expires_at_ledger, executed.
    """
    source_account = server.load_account(keypair.public_key)
    tx = (
        TransactionBuilder(
            source_account=source_account,
            network_passphrase=config.network_passphrase,
            base_fee=300,
        )
        .append_invoke_contract_function_op(
            contract_id=config.contract_id,
            function_name="get_operation",
            parameters=[scval.to_uint64(operation_id)],
        )
        .build()
    )
    sim = server.simulate_transaction(tx)
    if sim.error:
        raise RuntimeError(f"Simulation error: {sim.error}")
    raw = sim.results[0].xdr
    struct = scval.from_map(scval.from_xdr(raw))
    return {
        "id": scval.from_uint64(struct[scval.to_symbol("id")]),
        "kind": scval.from_symbol(struct[scval.to_symbol("kind")]),
        "args": scval.from_string(struct[scval.to_symbol("args")]),
        "created_at_ledger": scval.from_uint32(
            struct[scval.to_symbol("created_at_ledger")]
        ),
        "expires_at_ledger": scval.from_uint32(
            struct[scval.to_symbol("expires_at_ledger")]
        ),
        "executed": scval.from_bool(struct[scval.to_symbol("executed")]),
    }


def submit_execute_operation(
    server: SorobanServer,
    config: ExecutorConfig,
    keypair: Keypair,
    operation_id: int,
) -> str:
    """
    Submit execute_operation(id) transaction to the network.
    Returns the transaction hash on success.
    """
    source_account = server.load_account(keypair.public_key)
    tx = (
        TransactionBuilder(
            source_account=source_account,
            network_passphrase=config.network_passphrase,
            base_fee=300,
        )
        .append_invoke_contract_function_op(
            contract_id=config.contract_id,
            function_name="execute_operation",
            parameters=[scval.to_uint64(operation_id)],
        )
        .set_timeout(30)
        .build()
    )
    # Simulate to get auth + resource footprint
    sim = server.simulate_transaction(tx)
    if sim.error:
        raise RuntimeError(f"Simulation error before execution: {sim.error}")

    # Attach auth and resource fees
    tx = server.prepare_transaction(tx, sim)
    tx.sign(keypair)
    response = server.send_transaction(tx)
    if response.status == "ERROR":
        raise RuntimeError(f"Transaction rejected: {response.error_result_xdr}")

    # Poll for confirmation
    tx_hash = response.hash
    for _ in range(30):
        time.sleep(2)
        result = server.get_transaction(tx_hash)
        if result.status == "SUCCESS":
            return tx_hash
        if result.status == "FAILED":
            raise RuntimeError(
                f"Transaction {tx_hash} FAILED: {result.result_xdr}"
            )
    raise TimeoutError(f"Transaction {tx_hash} did not confirm within 60s")


def submit_expire_stale(
    server: SorobanServer,
    config: ExecutorConfig,
    keypair: Keypair,
) -> str:
    """
    Call expire_stale_operations() — open to anyone, not admin-gated.
    Returns the transaction hash.
    """
    source_account = server.load_account(keypair.public_key)
    tx = (
        TransactionBuilder(
            source_account=source_account,
            network_passphrase=config.network_passphrase,
            base_fee=300,
        )
        .append_invoke_contract_function_op(
            contract_id=config.contract_id,
            function_name="expire_stale_operations",
            parameters=[],
        )
        .set_timeout(30)
        .build()
    )
    sim = server.simulate_transaction(tx)
    if sim.error:
        raise RuntimeError(f"Simulation error: {sim.error}")
    tx = server.prepare_transaction(tx, sim)
    tx.sign(keypair)
    response = server.send_transaction(tx)
    if response.status == "ERROR":
        raise RuntimeError(f"Transaction rejected: {response.error_result_xdr}")
    tx_hash = response.hash
    for _ in range(30):
        time.sleep(2)
        result = server.get_transaction(tx_hash)
        if result.status == "SUCCESS":
            return tx_hash
        if result.status == "FAILED":
            raise RuntimeError(
                f"Transaction {tx_hash} FAILED: {result.result_xdr}"
            )
    raise TimeoutError(f"Transaction {tx_hash} did not confirm within 60s")


# ---------------------------------------------------------------------------
# Event monitor — incremental cursor-based polling
# ---------------------------------------------------------------------------

def poll_operation_queued_events(
    server: SorobanServer,
    config: ExecutorConfig,
) -> List[Dict[str, Any]]:
    """
    Poll for OperationQueuedEvent events since the last cursor.
    Updates config._event_cursor on each successful call.
    Returns a list of event dicts with keys: operation_id, expires_at_ledger.
    """
    filters = [
        EventFilter(
            event_type="contract",
            contract_ids=[config.contract_id],
            topics=[["OperationQueued"]],
        )
    ]
    req = GetEventsRequest(
        start_ledger=None,
        filters=filters,
        cursor=config._event_cursor,
        limit=200,
    )
    try:
        resp = server.get_events(req)
    except Exception as exc:  # noqa: BLE001
        log.warning("Event polling failed: %s", exc)
        return []

    events: List[Dict[str, Any]] = []
    for ev in resp.events:
        try:
            topics = ev.topic
            operation_id = scval.from_uint64(scval.from_xdr(topics[1]))
            body = scval.from_map(scval.from_xdr(ev.value.xdr))
            expires_at = scval.from_uint32(
                body[scval.to_symbol("expires_at_ledger")]
            )
            events.append(
                {"operation_id": operation_id, "expires_at_ledger": expires_at}
            )
        except Exception as exc:  # noqa: BLE001
            log.debug("Could not decode event: %s", exc)

    if resp.events:
        config._event_cursor = resp.events[-1].paging_token

    return events


# ---------------------------------------------------------------------------
# Core execution loop
# ---------------------------------------------------------------------------

class OperationExecutor:
    """
    Stateful executor that:
    1. Polls for OperationQueuedEvents to discover new pending operations.
    2. Fetches each pending op's details to determine eligibility.
    3. Auto-executes eligible ops (past grace period, not yet expired).
    4. Periodically sweeps stale expired operations via expire_stale_operations().
    5. Retries failures with exponential back-off and fires alerts on exhaustion.
    """

    EXPIRE_SWEEP_INTERVAL_POLLS = 10  # run sweep every N poll cycles

    def __init__(self, config: ExecutorConfig) -> None:
        self.config = config
        self.keypair = Keypair.from_secret(config.admin_secret_key)
        self.server = build_soroban_server(config)
        # Tracks retry counts per operation id
        self._retry_counts: Dict[int, int] = {}
        # Tracks permanently failed operation ids (do not retry further)
        self._failed_ops: set = set()
        self._poll_count = 0

    # ---- Public entry point ----

    def run_forever(self) -> None:
        log.info(
            "Executor started — contract=%s network=%s poll_interval=%.1fs",
            self.config.contract_id,
            self.config.network,
            self.config.poll_interval_secs,
        )
        while True:
            try:
                self._poll_cycle()
            except KeyboardInterrupt:
                log.info("Interrupted by user, shutting down.")
                break
            except Exception as exc:  # noqa: BLE001
                log.error("Unexpected error in poll cycle: %s\n%s", exc, traceback.format_exc())
            time.sleep(self.config.poll_interval_secs)

    # ---- Internal helpers ----

    def _poll_cycle(self) -> None:
        self._poll_count += 1
        log.debug("Poll cycle #%d", self._poll_count)

        current_ledger = self._get_current_ledger_safe()
        if current_ledger is None:
            return

        # 1. Discover new ops via events
        new_events = poll_operation_queued_events(self.server, self.config)
        for ev in new_events:
            log.info(
                "Discovered queued operation id=%d expires_at_ledger=%d",
                ev["operation_id"],
                ev["expires_at_ledger"],
            )

        # 2. Fetch all currently pending ids directly from contract state
        try:
            pending_ids = with_retry(
                call_contract_get_ids,
                self.server,
                self.config,
                self.keypair,
                max_attempts=self.config.max_retry_attempts,
                label="get_pending_operation_ids",
            )
        except RuntimeError as exc:
            log.error("Cannot fetch pending ids: %s", exc)
            return

        log.debug("Pending operation ids: %s", pending_ids)

        for op_id in pending_ids:
            if op_id in self._failed_ops:
                continue
            self._maybe_execute(op_id, current_ledger)

        # 3. Periodic stale sweep
        if self._poll_count % self.EXPIRE_SWEEP_INTERVAL_POLLS == 0:
            self._sweep_stale()

    def _maybe_execute(self, op_id: int, current_ledger: int) -> None:
        """Fetch operation details and execute if eligible."""
        try:
            op = with_retry(
                call_contract_get_operation,
                self.server,
                self.config,
                self.keypair,
                op_id,
                max_attempts=self.config.max_retry_attempts,
                label=f"get_operation({op_id})",
            )
        except RuntimeError as exc:
            log.warning("Could not fetch op %d: %s", op_id, exc)
            return

        if op["executed"]:
            log.debug("Op %d already executed, skipping.", op_id)
            return

        # Check grace period
        eligible_at = op["created_at_ledger"] + self.config.grace_ledgers
        if current_ledger < eligible_at:
            log.debug(
                "Op %d not yet eligible (current=%d, eligible_at=%d).",
                op_id,
                current_ledger,
                eligible_at,
            )
            return

        # Check expiry
        if current_ledger > op["expires_at_ledger"]:
            log.info(
                "Op %d is already expired (current=%d, expires=%d). "
                "Will be swept on next maintenance cycle.",
                op_id,
                current_ledger,
                op["expires_at_ledger"],
            )
            return

        log.info(
            "Executing op id=%d kind=%s (created=%d, expires=%d, current=%d)",
            op_id,
            op["kind"],
            op["created_at_ledger"],
            op["expires_at_ledger"],
            current_ledger,
        )
        self._execute_with_retry(op_id, op)

    def _execute_with_retry(self, op_id: int, op: Dict[str, Any]) -> None:
        attempt = self._retry_counts.get(op_id, 0) + 1
        self._retry_counts[op_id] = attempt
        try:
            tx_hash = submit_execute_operation(
                self.server, self.config, self.keypair, op_id
            )
            log.info("✓ Op %d executed — tx_hash=%s", op_id, tx_hash)
            # Clean up tracking state on success
            self._retry_counts.pop(op_id, None)
        except Exception as exc:  # noqa: BLE001
            log.warning(
                "Execution attempt %d/%d for op %d failed: %s",
                attempt,
                self.config.max_retry_attempts,
                op_id,
                exc,
            )
            if attempt >= self.config.max_retry_attempts:
                self._failed_ops.add(op_id)
                send_alert(
                    self.config,
                    subject=f"Execution permanently failed for op {op_id}",
                    body=(
                        f"Operation id={op_id} kind={op.get('kind')} "
                        f"args={op.get('args')} failed after "
                        f"{self.config.max_retry_attempts} attempts.\n"
                        f"Last error: {exc}"
                    ),
                )

    def _sweep_stale(self) -> None:
        log.info("Running expire_stale_operations() maintenance sweep…")
        try:
            tx_hash = with_retry(
                submit_expire_stale,
                self.server,
                self.config,
                self.keypair,
                max_attempts=3,
                label="expire_stale_operations",
            )
            log.info("✓ Stale sweep complete — tx_hash=%s", tx_hash)
        except Exception as exc:  # noqa: BLE001
            log.warning("Stale sweep failed (non-critical): %s", exc)

    def _get_current_ledger_safe(self) -> Optional[int]:
        try:
            return with_retry(
                get_current_ledger,
                self.server,
                max_attempts=3,
                label="get_current_ledger",
            )
        except Exception as exc:  # noqa: BLE001
            log.error("Cannot fetch current ledger: %s", exc)
            return None


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def main() -> None:
    import argparse

    parser = argparse.ArgumentParser(
        description="Price Oracle off-chain executor service"
    )
    parser.add_argument(
        "--config",
        default=None,
        metavar="PATH",
        help="Path to JSON config file (optional; env vars take precedence)",
    )
    args = parser.parse_args()

    config = ExecutorConfig.from_env_and_file(args.config)
    executor = OperationExecutor(config)
    executor.run_forever()


if __name__ == "__main__":
    main()
