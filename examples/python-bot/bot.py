"""Minimal price-submission bot: builds, simulates and submits `submit_price`."""
import os
import time

from stellar_sdk import Keypair, Network, SorobanServer, TransactionBuilder, scval
from stellar_sdk.account import Account

DECIMALS = 7


def build_submit(source: Keypair, contract_id: str, asset: str, price: float, sequence: int, ts: int,
                 passphrase: str = Network.TESTNET_NETWORK_PASSPHRASE):
    if price <= 0:
        raise ValueError("price must be > 0")  # contract would return InvalidPrice (7)
    args = [
        scval.to_address(source.public_key),
        scval.to_address(asset),
        scval.to_int128(round(price * 10**DECIMALS)),
        scval.to_uint64(ts),
    ]
    return (
        TransactionBuilder(Account(source.public_key, sequence), passphrase, base_fee=100)
        .append_invoke_contract_function_op(contract_id, "submit_price", args)
        .set_timeout(30)
        .build()
    )


def main():
    # Secret comes from the environment / secret manager only. Never hard-code or commit it.
    kp = Keypair.from_secret(os.environ["ORACLE_SOURCE_SECRET"])
    server = SorobanServer(os.environ.get("RPC_URL", "https://soroban-testnet.stellar.org"))
    acct = server.load_account(kp.public_key)
    tx = build_submit(kp, os.environ["CONTRACT_ID"], os.environ["ASSET_ADDRESS"], float(os.environ["PRICE"]),
                      acct.sequence, int(time.time()))
    tx = server.prepare_transaction(tx)
    tx.sign(kp)
    print(server.send_transaction(tx))


if __name__ == "__main__":
    main()
