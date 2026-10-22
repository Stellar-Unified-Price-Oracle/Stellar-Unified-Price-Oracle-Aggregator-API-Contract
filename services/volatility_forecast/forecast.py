"""Projects short-term price volatility from an asset's indexed aggregate
price history, for use as risk parameters (deviation thresholds,
circuit-breaker bounds) by consumers and the DAO.

Two volatility measures are computed from the same log-return series:

* **Realized volatility** — the plain historical stdev of log returns over
  the lookback window. Backward-looking: "how volatile has this asset
  actually been."
* **EWMA (forward) volatility** — an exponentially-weighted variance
  (RiskMetrics-style, decay `lambda_`) that reacts faster to a recent
  regime change than the flat realized-vol average. This is used as the
  forward-looking forecast basis. The oracle has no options market to
  derive a true *implied* volatility from, so this EWMA projection is the
  documented stand-in: it weights recent moves more heavily, the same way
  an options market's implied vol tends to lead realized vol into a shift.

The forecast projects the EWMA volatility over a horizon and reports a
confidence window as a symmetric price band around the last observed price,
assuming a lognormal random walk (the same assumption behind Black-Scholes-
style vol scaling: sigma scales with sqrt(time)).
"""
from __future__ import annotations

import math
import statistics
from dataclasses import dataclass
from typing import List, Optional, Sequence, Tuple

SECONDS_PER_YEAR = 365 * 24 * 3600
DEFAULT_EWMA_LAMBDA = 0.94  # RiskMetrics standard decay factor
DEFAULT_CONFIDENCE = 0.90


def log_returns(prices: Sequence[float]) -> List[float]:
    returns: List[float] = []
    for prev, cur in zip(prices, prices[1:]):
        if prev > 0 and cur > 0:
            returns.append(math.log(cur / prev))
    return returns


def realized_volatility(prices: Sequence[float], interval_secs: float, annualize: bool = True) -> float:
    """Plain historical stdev of log returns, optionally annualized."""
    returns = log_returns(prices)
    if len(returns) < 2:
        return 0.0
    period_vol = statistics.stdev(returns)
    return _maybe_annualize(period_vol, interval_secs, annualize)


def ewma_period_volatility(prices: Sequence[float], lambda_: float = DEFAULT_EWMA_LAMBDA) -> float:
    """Exponentially-weighted per-period volatility (not annualized) — the
    forward-looking forecast basis."""
    returns = log_returns(prices)
    if not returns:
        return 0.0
    variance = returns[0] ** 2
    for r in returns[1:]:
        variance = lambda_ * variance + (1.0 - lambda_) * r * r
    return math.sqrt(variance)


def ewma_volatility(
    prices: Sequence[float], interval_secs: float, lambda_: float = DEFAULT_EWMA_LAMBDA, annualize: bool = True
) -> float:
    return _maybe_annualize(ewma_period_volatility(prices, lambda_), interval_secs, annualize)


def _maybe_annualize(period_vol: float, interval_secs: float, annualize: bool) -> float:
    if not annualize or interval_secs <= 0:
        return period_vol
    return period_vol * math.sqrt(SECONDS_PER_YEAR / interval_secs)


def norm_ppf(p: float) -> float:
    """Inverse standard normal CDF (Acklam's rational approximation, ~1e-9
    relative error) — avoids a scipy dependency for confidence-window z-values."""
    if not 0.0 < p < 1.0:
        raise ValueError("p must be in (0, 1)")

    a = [-3.969683028665376e01, 2.209460984245205e02, -2.759285104469687e02,
         1.383577518672690e02, -3.066479806614716e01, 2.506628277459239e00]
    b = [-5.447609879822406e01, 1.615858368580409e02, -1.556989798598866e02,
         6.680131188771972e01, -1.328068155288572e01]
    c = [-7.784894002430293e-03, -3.223964580411365e-01, -2.400758277161838e00,
         -2.549732539343734e00, 4.374664141464968e00, 2.938163982698783e00]
    d = [7.784695709041462e-03, 3.224671290700398e-01, 2.445134137142996e00, 3.754408661907416e00]

    p_low, p_high = 0.02425, 1 - 0.02425

    if p < p_low:
        q = math.sqrt(-2 * math.log(p))
        return (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / (
            (((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1
        )
    if p <= p_high:
        q = p - 0.5
        r = q * q
        return (
            (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
        ) / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1)

    q = math.sqrt(-2 * math.log(1 - p))
    return -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / (
        (((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1
    )


def confidence_z(confidence: float) -> float:
    """Two-sided z-value for a confidence level, e.g. 0.90 -> ~1.645."""
    if not 0.0 < confidence < 1.0:
        raise ValueError("confidence must be in (0, 1)")
    return norm_ppf(0.5 + confidence / 2.0)


@dataclass
class VolatilityForecast:
    asset: str
    last_price: float
    realized_volatility: float  # annualized
    forecast_volatility: float  # annualized EWMA projection
    horizon_secs: int
    confidence: float
    lower_bound: float
    upper_bound: float
    sample_size: int


def forecast(
    asset: str,
    prices: Sequence[float],
    interval_secs: float,
    horizon_secs: int,
    confidence: float = DEFAULT_CONFIDENCE,
    lambda_: float = DEFAULT_EWMA_LAMBDA,
) -> Optional[VolatilityForecast]:
    """Builds a `VolatilityForecast` from a chronological price series.

    Returns `None` if there isn't enough history (fewer than 3 prices) to
    compute a meaningful volatility estimate.
    """
    if len(prices) < 3 or interval_secs <= 0 or horizon_secs <= 0:
        return None

    last_price = float(prices[-1])
    realized = realized_volatility(prices, interval_secs)
    ewma_period = ewma_period_volatility(prices, lambda_)
    forecast_vol = _maybe_annualize(ewma_period, interval_secs, True)

    n_periods = horizon_secs / interval_secs
    sigma_horizon = ewma_period * math.sqrt(n_periods)
    z = confidence_z(confidence)

    lower = last_price * math.exp(-z * sigma_horizon)
    upper = last_price * math.exp(z * sigma_horizon)

    return VolatilityForecast(
        asset=asset,
        last_price=last_price,
        realized_volatility=realized,
        forecast_volatility=forecast_vol,
        horizon_secs=horizon_secs,
        confidence=confidence,
        lower_bound=lower,
        upper_bound=upper,
        sample_size=len(prices),
    )
