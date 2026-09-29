"""Type-hinted Python SDK for the Stellar Unified Price Oracle contract."""
from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Optional

from stellar_sdk import Account, Keypair, Network, SorobanServer, TransactionBuilder, scval
from stellar_sdk.soroban_rpc import GetTransactionStatus


@dataclass
class PriceEntry:
    price: int
    timestamp: int


@dataclass
class OracleClientConfig:
    contract_id: str
    rpc_url: str
    network_passphrase: str = Network.TESTNET_NETWORK_PASSPHRASE


class OracleClient:
    """Auth-aware, typed client for the oracle contract's core endpoints."""

    def __init__(self, config: OracleClientConfig) -> None:
        self.config = config
        self.server = SorobanServer(config.rpc_url)

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
