from __future__ import annotations

import json

from services.soak.rig import (
    DEFAULT_MIX,
    LeakyStateModel,
    MixEntry,
    SoakConfig,
    StateModel,
    Thresholds,
    adversarial_fraction,
    build_workload,
    main,
    mix_weight,
    percentile,
    prometheus,
    render_report,
    run_soak,
)

import random


def test_mix_weights_are_normalised_and_adversarial_stays_a_minority():
    total = sum(e.weight for e in DEFAULT_MIX)
    assert abs(total - 1.0) < 1e-9
    # The mix must not stop resembling production: adversarial classes are
    # bounded to a third of the weight, and the routine class dominates.
    assert adversarial_fraction(DEFAULT_MIX) < 0.34
    assert mix_weight(DEFAULT_MIX, "routine") > 0.5
    assert adversarial_fraction(DEFAULT_MIX) > 0.1  # not trivially clean either


def test_percentiles_are_nearest_rank():
    xs = list(range(1, 101))
    assert percentile(xs, 50) == 50
    assert percentile(xs, 99) == 99
    assert percentile(xs, 100) == 100
    assert percentile([], 99) == 0.0


def test_bounded_state_model_is_bounded_by_the_window():
    state = StateModel(window=4, state_bytes=64)
    for i in range(500):
        state.apply("XLM", "SRC", i)
    # 4 retained entries x 64 B, no matter how many submissions arrived.
    assert state.bytes_retained == 4 * 64


def test_leaky_state_model_retains_everything():
    leaky = LeakyStateModel(window=4, state_bytes=64)
    for i in range(100):
        leaky.apply("XLM", "SRC", i)
    assert leaky.bytes_retained == 100 * 64
    assert leaky.leaky is True


def test_workload_is_deterministic_for_a_seed():
    a = list(build_workload(SoakConfig(rounds=25), random.Random(7)))
    b = list(build_workload(SoakConfig(rounds=25), random.Random(7)))
    assert [(s.asset, s.source, s.price, s.kind) for s in a] == [
        (s.asset, s.source, s.price, s.kind) for s in b
    ]


def test_workload_emits_every_mix_class():
    subs = list(build_workload(SoakConfig(rounds=400), random.Random(11)))
    assert {s.kind for s in subs} == {e.name for e in DEFAULT_MIX}


def test_soak_stays_within_all_thresholds():
    result = run_soak(SoakConfig(rounds=1_500))
    assert result.breaches == []
    assert result.passed
    p = result.latency_percentiles()
    assert 0 < p["p50"] <= p["p95"] <= p["p99"] <= p["max"]


def test_state_growth_is_measured_and_bounded():
    result = run_soak(SoakConfig(rounds=1_500))
    # A pruning window means the retained set stops growing: the fitted slope
    # over the post-warmup samples is zero, and the peak is small and constant.
    assert result.state_growth_per_1k() == 0.0
    expected = 7 * 3 * 8 * 64  # sources x assets x window x entry bytes
    assert max(result.state_samples) == expected
    assert max(result.state_samples) < result.config.thresholds.max_state_bytes


def test_memory_is_measured_over_the_run():
    result = run_soak(SoakConfig(rounds=300))
    assert len(result.memory_samples) == len(result.latencies)
    assert result.memory_peak() > 0
    assert result.memory_peak() <= result.config.thresholds.max_memory_bytes


def test_leaky_change_is_detected_not_silently_passed():
    """A deliberately leaky state model must trip the growth assertion."""
    result = run_soak(SoakConfig(rounds=1_500, leaky=True))
    assert not result.passed
    assert any("not bounded" in b for b in result.breaches)
    # Detected by growth, and the leak also shows up as latency drift.
    assert result.state_growth_per_1k() == 64 * 1_000
    assert any("drift" in b for b in result.breaches)


def test_leak_detection_is_robust_across_seeds():
    for seed in (1, 42, 999, 20260928):
        assert run_soak(SoakConfig(rounds=800, seed=seed, leaky=True)).breaches
        assert run_soak(SoakConfig(rounds=800, seed=seed)).breaches == []


def test_latency_ceiling_is_asserted():
    tight = Thresholds(max_p99_latency_ms=0.5)
    result = run_soak(SoakConfig(rounds=200, thresholds=tight))
    assert any("p99" in b for b in result.breaches)


def test_report_and_metrics_expose_the_measured_quantities():
    result = run_soak(SoakConfig(rounds=300))
    report = render_report(result)
    assert report.startswith("# Soak report — PASS")
    for metric in ("latency p50", "latency p99", "state growth", "memory peak"):
        assert metric in report
    text = prometheus(result)
    assert 'soak_latency_ms{quantile="p99",model="bounded"}' in text
    assert "soak_threshold_breaches{model=\"bounded\"} 0" in text


def test_cli_writes_artifacts_and_signals_failure(tmp_path, capsys):
    ok = main([
        "--rounds", "300",
        "--json", str(tmp_path / "r.json"),
        "--report", str(tmp_path / "r.md"),
        "--prometheus", str(tmp_path / "r.prom"),
    ])
    assert ok == 0
    payload = json.loads((tmp_path / "r.json").read_text())
    assert payload["state_model"] == "bounded" and payload["breaches"] == []
    assert (tmp_path / "r.md").read_text().startswith("# Soak report")
    assert "soak_latency_ms" in (tmp_path / "r.prom").read_text()

    # The leak drill: the CLI exits 0 only because a breach was expected.
    assert main(["--rounds", "800", "--model", "leaky", "--expect-breach"]) == 0
    assert main(["--rounds", "300", "--model", "leaky"]) == 1


def test_adversarial_mix_does_not_dominate_the_run():
    result = run_soak(SoakConfig(rounds=1_500))
    total = sum(result.kind_counts.values())
    assert result.submissions == total
    realistic = mix_weight(DEFAULT_MIX, "routine") + mix_weight(DEFAULT_MIX, "diurnal_burst")
    assert sum(result.kind_counts[k] for k in ("routine", "diurnal_burst")) / total > 0.6


def test_unbounded_history_window_is_configurable():
    """A larger window is a larger but still constant retained set."""
    result = run_soak(SoakConfig(rounds=600, history_window=32))
    assert result.state_growth_per_1k() == 0.0
    assert max(result.state_samples) == 7 * 3 * 32 * 64
