"""Type-hinted Python SDK for the Stellar Unified Price Oracle contract."""
from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Any, Dict, Optional

from stellar_sdk import Account, Keypair, Network, SorobanServer, TransactionBuilder, scval
from stellar_sdk.soroban_rpc import GetTransactionStatus

from .generated_bindings import OracleBindings


@dataclass
class PriceEntry:
    price: int
    timestamp: int


@dataclass
class OracleClientConfig:
    contract_id: str
    rpc_url: str
    network_passphrase: str = Network.TESTNET_NETWORK_PASSPHRASE


class OracleError(Exception):
    """Base typed error representing a known oracle contract error (#873)."""

    def __init__(self, code: int, name: str, meaning: str, remediation: str) -> None:
        super().__init__(f"[{name} (#{code})]: {meaning} — {remediation}")
        self.code = code
        self.name = name
        self.meaning = meaning
        self.remediation = remediation


def map_error_code(code: int) -> OracleError:
    """Map integer error code to typed OracleError (#873)."""
    error_map: Dict[int, Dict[str, str]] = {
        0: {"name": "NotAuthorized", "meaning": "Caller not authorized.", "remediation": "Check admin/source signature."},
        1: {"name": "AlreadyInitialized", "meaning": "Contract already initialized.", "remediation": "Do not re-initialize."},
        2: {"name": "AssetNotRegistered", "meaning": "Asset is not registered.", "remediation": "Register asset first."},
        3: {"name": "AssetAlreadyRegistered", "meaning": "Asset already registered.", "remediation": "Treat as success."},
        4: {"name": "SourceAlreadyExists", "meaning": "Source already exists.", "remediation": "Do not duplicate."},
        5: {"name": "SourceNotFound", "meaning": "Source address not found.", "remediation": "Register source first."},
        6: {"name": "SourceNotActive", "meaning": "Source is inactive.", "remediation": "Reactivate source."},
        7: {"name": "InvalidPrice", "meaning": "Price must be strictly positive.", "remediation": "Submit positive price."},
        8: {"name": "InvalidTimestamp", "meaning": "Timestamp outside acceptable threshold.", "remediation": "Use current unix time."},
        9: {"name": "InsufficientSources", "meaning": "Fewer contributing sources than quorum.", "remediation": "Ensure min sources submit."},
        58: {"name": "ContractPaused", "meaning": "Contract paused.", "remediation": "Wait for unpause."},
        168: {"name": "SimulationRequired", "meaning": "Simulation required.", "remediation": "Attach simulation."},
        169: {"name": "ProposalHarmful", "meaning": "Proposal flagged harmful.", "remediation": "Revise proposal."},
    }

    info = error_map.get(
        code,
        {
            "name": "UnknownError",
            "meaning": f"Contract error code {code}",
            "remediation": "Refer to docs/errors/registry.json",
        },
    )
    return OracleError(code=code, **info)


def to_decimal(price: int, decimals: int) -> float:
    """Convert raw integer price into scaled decimal float (#873)."""
    return price / (10 ** decimals)


class OracleClient:
    """Auth-aware, typed client for the oracle contract's core endpoints."""

    def __init__(self, config: OracleClientConfig) -> None:
        self.config = config
        self.server = SorobanServer(config.rpc_url)
        # Generated raw bindings wrapper (#871)
        self.bindings = OracleBindings(self._invoke_untyped)

    def _invoke_untyped(self, method: str, args: list) -> Any:
        return self._view(method, args)

    def _invoke(self, method: str, args: list, signer: Keypair):
        source = self.server.load_account(signer.public_key)
        tx = (
            TransactionBuilder(source, self.config.network_passphrase, base_fee=100_000)
            .add_time_bounds(0, 0)
            .append_invoke_contract_function_op(self.config.contract_id, method, args)
            .build()
        )
        prepared = self.server.prepare_transaction(tx)
        prepared.sign(signer)
        send_result = self.server.send_transaction(prepared)

        status = send_result.status
        for _ in range(10):
            if status != "PENDING":
                break
            time.sleep(1)
            result = self.server.get_transaction(send_result.hash)
            status = result.status
            if status == GetTransactionStatus.SUCCESS:
                return result.return_value
        if status != GetTransactionStatus.SUCCESS:
            raise RuntimeError(f"transaction {send_result.hash} failed with status {status}")

    def _view(self, method: str, args: list):
        # Simulation doesn't require a funded/valid account; sequence number is unused.
        source = Account(Keypair.random().public_key, 0)
        tx = (
            TransactionBuilder(source, self.config.network_passphrase, base_fee=100)
            .add_time_bounds(0, 0)
            .append_invoke_contract_function_op(self.config.contract_id, method, args)
            .build()
        )
        sim = self.server.simulate_transaction(tx)
        if sim.error:
            raise RuntimeError(sim.error)
        return sim.results[0].xdr if sim.results else None

    # ── Price queries ────────────────────────────────────────────────
    def get_price(self, asset: str, max_age: int) -> Optional[PriceEntry]:
        return self._view("get_price", [scval.to_address(asset), scval.to_uint64(max_age)])

    def get_fresh_price(self, asset: str, max_age_sec: int) -> Optional[PriceEntry]:
        """Staleness-checked price lookup (#873)."""
        entry = self.get_price(asset, max_age_sec)
        if not entry:
            return None
        now = int(time.time())
        if now > entry.timestamp and now - entry.timestamp > max_age_sec:
            return None
        return entry

    def get_source_price(self, asset: str, source: str) -> PriceEntry:
        return self._view("get_source_price", [scval.to_address(asset), scval.to_address(source)])

    def get_all_prices(self, asset: str) -> list[PriceEntry]:
        return self._view("get_all_prices", [scval.to_address(asset)])

    # ── Submission ──────────────────────────────────────────────────
    def submit_price(self, source: str, asset: str, price: int, timestamp: int, signer: Keypair):
        return self._invoke(
            "submit_price",
            [scval.to_address(source), scval.to_address(asset), scval.to_int128(price), scval.to_uint64(timestamp)],
            signer,
        )

    # ── Subscription management ────────────────────────────────────
    def subscribe(self, consumer: str, duration: int, signer: Keypair):
        return self._invoke("subscribe", [scval.to_address(consumer), scval.to_uint32(duration)], signer)

    def renew_subscription(self, consumer: str, signer: Keypair):
        return self._invoke("renew_subscription", [scval.to_address(consumer)], signer)

    def get_subscription_expiry(self, consumer: str) -> int:
        return self._view("get_subscription_expiry", [scval.to_address(consumer)])

    # ── Source / asset registry ────────────────────────────────────
    def is_source(self, source: str) -> bool:
        return self._view("is_source", [scval.to_address(source)])

    def is_asset_registered(self, asset: str) -> bool:
        return self._view("is_asset_registered", [scval.to_address(asset)])
