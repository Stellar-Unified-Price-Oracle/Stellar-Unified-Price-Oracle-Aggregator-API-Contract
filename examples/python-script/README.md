# Python Consumer Script Quickstart (5-Minute Guide)

This quickstart demonstrates how to query the Stellar Unified Price Oracle from Python using `stellar-sdk`.

## Quick Example

```python
from stellar_sdk import SorobanServer
from read_price import query_price

server = SorobanServer("https://soroban-testnet.stellar.org")
price_info = query_price(server, contract_id="C...", asset_address="C...")
print(price_info)
```

## Running Tests

```bash
pip install -r requirements.txt
pytest
```
