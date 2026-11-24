# Testnet Explorer

The static explorer is deployed by `.github/workflows/explorer-pages.yml` to
GitHub Pages. Set the repository variable `ORACLE_TESTNET_CONTRACT_ID` to the
testnet contract ID before deployment. The testnet Soroban RPC endpoint is
configured by default; local copies can edit `runtime-config.js`.

The **Live Feed** panel reads the aggregate price, source submissions, contract
pause state, and asset pause state for an entered asset address. Freshness is
reported against a five-minute window. The existing request builder remains
available for direct read and invoke calls.
