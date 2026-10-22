import pytest
from stellar_sdk import Keypair, StrKey

from bot import build_submit

CONTRACT = StrKey.encode_contract(bytes(32))
ASSET = StrKey.encode_contract(bytes([1]) * 32)


def test_builds_signed_submit_offline():
    kp = Keypair.random()  # ephemeral key; real keys come from env
    tx = build_submit(kp, CONTRACT, ASSET, 1234.5, 1, 1700000000)
    tx.sign(kp)
    op = tx.transaction.operations[0]
    assert op.host_function.invoke_contract.function_name.sc_symbol == b"submit_price"
    assert len(tx.signatures) == 1


def test_rejects_non_positive_price():
    with pytest.raises(ValueError):
        build_submit(Keypair.random(), CONTRACT, ASSET, 0, 1, 1700000000)
