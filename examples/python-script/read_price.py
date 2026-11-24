"""Quickstart: 5-minute Python script consuming price data from SEP-40 Oracle."""

import os
from typing import Any, Dict, Optional
from stellar_sdk import Network, SorobanServer, TransactionBuilder, scval, xdr
from stellar_sdk.account import Account

READ_ONLY_SOURCE = "GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7"


def format_price_result(raw_price: int, decimals: int, timestamp: int) -> Dict[str, Any]:
    return {
        "price": raw_price / (10 ** decimals),
        "raw_price": raw_price,
        "decimals": decimals,
        "timestamp": timestamp,
    }


def query_price(
    server: SorobanServer,
    contract_id: str,
    asset_address: str,
    passphrase: str = Network.TESTNET_NETWORK_PASSPHRASE,
) -> Optional[Dict[str, Any]]:
    # 1. Read contract decimals
    dec_tx = (
        TransactionBuilder(Account(READ_ONLY_SOURCE, 0), passphrase, base_fee=100)
        .append_invoke_contract_function_op(contract_id, "decimals", [])
        .set_timeout(30)
        .build()
    )
    dec_sim = server.simulate_transaction(dec_tx)
    if dec_sim.error:
        raise RuntimeError(f"Simulation error fetching decimals: {dec_sim.error}")
    decimals = scval.from_scval(dec_sim.results[0].xdr).value

    # 2. Read SEP-40 lastprice
    sep40_asset = xdr.SCVal(
        xdr.SCValType.SCV_VEC,
        vec=[scval.to_symbol("Stellar"), scval.to_address(asset_address)],
    )
    price_tx = (
        TransactionBuilder(Account(READ_ONLY_SOURCE, 0), passphrase, base_fee=100)
        .append_invoke_contract_function_op(contract_id, "lastprice", [sep40_asset])
        .set_timeout(30)
        .build()
    )
    price_sim = server.simulate_transaction(price_tx)
    if price_sim.error:
        raise RuntimeError(f"Simulation error fetching price: {price_sim.error}")
    res = scval.from_scval(price_sim.results[0].xdr)
    if res is None:
        return None

    return format_price_result(res["price"], decimals, res["timestamp"])


if __name__ == "__main__":
    cid = os.environ.get("CONTRACT_ID", "")
    asset = os.environ.get("ASSET_ADDRESS", "")
    if not cid or not asset:
        print("Usage: CONTRACT_ID=... ASSET_ADDRESS=... python read_price.py")
    else:
        srv = SorobanServer(os.environ.get("RPC_URL", "https://soroban-testnet.stellar.org"))
        print(query_price(srv, cid, asset))
