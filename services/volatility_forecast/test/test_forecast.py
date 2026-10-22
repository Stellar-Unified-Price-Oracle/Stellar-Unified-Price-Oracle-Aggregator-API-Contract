from __future__ import annotations

import math

import pytest

from services.common.events import iter_aggregations
from services.common.synthetic import generate_price_series
from services.volatility_forecast.forecast import (
    confidence_z,
    ewma_volatility,
    forecast,
    log_returns,
    norm_ppf,
    realized_volatility,
)


def _prices(events):
    return [e.price for e in iter_aggregations(events)]


def test_log_returns_basic():
    returns = log_returns([100.0, 110.0, 99.0])
    assert returns == pytest.approx([math.log(1.1), math.log(99 / 110)])


def test_realized_volatility_recovers_known_input_stdev():
    # A hand-built series alternating +1%/-1% moves has a log-return series
    # of exactly [+ln(1.01), -ln(1.01), ...] — an independently computable
    # ground truth this test checks the implementation against.
    import statistics as _statistics

    prices = [100.0]
    for i in range(20):
        prices.append(prices[-1] * (1.01 if i % 2 == 0 else 1 / 1.01))
    expected = _statistics.stdev([math.log(1.01) if i % 2 == 0 else -math.log(1.01) for i in range(20)])

    period_vol = realized_volatility(prices, interval_secs=3600, annualize=False)
    assert period_vol == pytest.approx(expected, rel=1e-9)


def test_realized_volatility_scales_with_annualization():
    prices = [100.0 * (1.001**i) for i in range(50)]
    period = realized_volatility(prices, interval_secs=3600, annualize=False)
    annual = realized_volatility(prices, interval_secs=3600, annualize=True)
    periods_per_year = (365 * 24 * 3600) / 3600
    assert annual == pytest.approx(period * math.sqrt(periods_per_year))


def test_volatile_series_has_higher_forecast_vol_than_calm_series():
    calm = _prices(generate_price_series(n_points=200, period_volatility=0.002, seed=1))
    volatile = _prices(generate_price_series(n_points=200, period_volatility=0.05, seed=1))

    calm_vol = ewma_volatility(calm, interval_secs=3600)
    volatile_vol = ewma_volatility(volatile, interval_secs=3600)

    assert volatile_vol > calm_vol


def test_confidence_z_matches_known_values():
    assert confidence_z(0.90) == pytest.approx(1.645, abs=1e-3)
    assert confidence_z(0.95) == pytest.approx(1.960, abs=1e-3)
    assert confidence_z(0.99) == pytest.approx(2.576, abs=1e-3)


def test_norm_ppf_is_symmetric_around_the_median():
    assert norm_ppf(0.5) == pytest.approx(0.0, abs=1e-9)
    assert norm_ppf(0.25) == pytest.approx(-norm_ppf(0.75), abs=1e-9)


def test_forecast_confidence_window_widens_with_confidence_level():
    prices = _prices(generate_price_series(n_points=300, period_volatility=0.01, seed=5))
    f90 = forecast("asset", prices, interval_secs=3600, horizon_secs=3600 * 24, confidence=0.90)
    f99 = forecast("asset", prices, interval_secs=3600, horizon_secs=3600 * 24, confidence=0.99)

    assert (f99.upper_bound - f99.lower_bound) > (f90.upper_bound - f90.lower_bound)


def test_forecast_confidence_window_widens_with_horizon():
    prices = _prices(generate_price_series(n_points=300, period_volatility=0.01, seed=6))
    short = forecast("asset", prices, interval_secs=3600, horizon_secs=3600, confidence=0.90)
    long = forecast("asset", prices, interval_secs=3600, horizon_secs=3600 * 24 * 7, confidence=0.90)

    assert (long.upper_bound - long.lower_bound) > (short.upper_bound - short.lower_bound)


def test_forecast_bounds_are_centered_on_the_last_price_in_log_space():
    prices = _prices(generate_price_series(n_points=300, period_volatility=0.01, seed=7))
    f = forecast("asset", prices, interval_secs=3600, horizon_secs=3600 * 24, confidence=0.90)
    assert math.log(f.upper_bound / f.last_price) == pytest.approx(-math.log(f.lower_bound / f.last_price))


def test_forecast_returns_none_with_insufficient_history():
    assert forecast("asset", [100.0, 101.0], interval_secs=3600, horizon_secs=3600) is None


def test_forecast_end_to_end_reports_sample_size_and_asset():
    events = generate_price_series(n_points=100, seed=9)
    prices = _prices(events)
    f = forecast("MY_ASSET", prices, interval_secs=3600, horizon_secs=3600 * 6)
    assert f.asset == "MY_ASSET"
    assert f.sample_size == 100
    assert f.last_price == prices[-1]
    assert f.lower_bound < f.last_price < f.upper_bound
