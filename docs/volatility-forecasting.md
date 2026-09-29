# Off-Chain Volatility Forecasting Service

Projects short-term price volatility for an asset from its indexed aggregate price history.
Consumers and the DAO use the forecast to set risk parameters — deviation thresholds,
circuit-breaker bounds — ahead of time rather than reactively.

Code: `services/volatility_forecast/` (`forecast.py` — volatility math, `server.py` — HTTP
forecast endpoint + dashboard).

## Method

Both volatility measures below are computed from the same log-return series of an asset's
aggregate price history (`PriceAggregatedEvent`, read via the indexed `oracle_events` stream —
see `docs/event-streaming/README.md`):

- **Realized volatility** — the plain historical stdev of log returns over the full lookback
  window, annualized. Backward-looking: how volatile the asset has actually been.
- **Forecast volatility** — an EWMA (RiskMetrics-style, λ=0.94 by default) variance estimate,
  annualized. Weights recent moves more heavily than old ones, so it reacts to a regime change
  faster than the flat realized-vol average. The oracle has no options market to derive a true
  *implied* volatility from; this EWMA projection is the documented stand-in used as the
  forward-looking forecast basis, the same way an options market's implied vol tends to lead
  realized vol into a shift.

The **confidence window** projects the forecast volatility over the requested horizon and
reports a symmetric price band around the last observed price, assuming a lognormal random
walk (volatility scales with `sqrt(horizon / interval)`, the same assumption behind
Black-Scholes-style vol scaling). The z-value for an arbitrary confidence level is computed
via `norm_ppf` (Acklam's rational approximation of the inverse normal CDF) — no `scipy`
dependency required.

## Running

```bash
pip install -r services/volatility_forecast/requirements.txt

# Demo: serves the dashboard + forecast endpoint against a synthetic price series.
python -m services.volatility_forecast.server --demo
# then open http://localhost:9102/ or:
curl 'http://localhost:9102/forecast?asset=DEMO&horizon_hours=24&confidence=0.90'

# Against a real exported history:
python -m services.volatility_forecast.server --events-file events.jsonl --interval-secs 3600
```

## HTTP endpoints

The service listens on `--port` (default `9102`):

| Path | Description |
|------|-------------|
| `/` | Dashboard view — pick an asset, horizon, and confidence level; renders the forecast |
| `/forecast?asset=&horizon_hours=&confidence=` | JSON forecast: `realized_volatility`, `forecast_volatility`, `lower_bound`, `upper_bound`, `sample_size` |
| `/metrics` | Prometheus counter: `oracle_volatility_forecasts_total` |
| `/health` | Liveness check |

Add `/metrics` as a Prometheus scrape target alongside the other oracle off-chain services
(see `docs/monitoring/README.md`); `docs/monitoring/alerts.yml` includes a staleness rule for
this service.

## Testing

```bash
python -m pytest services/volatility_forecast/test/
```

Covers the volatility math against independently-derived ground truth (a hand-built
alternating-return series with a known stdev, annualization scaling), the confidence-window
z-values against textbook normal-quantile values, that the window widens with confidence level
and horizon as expected, and an end-to-end HTTP integration test against a running server
instance.
