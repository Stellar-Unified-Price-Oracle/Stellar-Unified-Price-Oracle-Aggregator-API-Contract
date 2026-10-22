#!/usr/bin/env python3
"""Unit tests for the pure event -> metric derivation logic in metrics_exporter.py.

Run with: python3 -m unittest scripts/test_metrics_exporter.py -v

These exercise EventIndex directly with synthetic events (no network/RPC),
proving the freshness, deviation, and source-health metrics the v2 alert
rules (docs/monitoring/alerts-v2.yml) depend on are computed correctly.
"""
import unittest

from metrics_exporter import EventIndex


class TestEventIndex(unittest.TestCase):
    def test_freshness_metric_tracks_latest_update(self):
        idx = EventIndex("CTEST")
        idx.handle_event(["PriceUpdated", "USDC"], {"new_price": 100_000}, ledger_close_time=1_000)
        self.assertEqual(idx.latest_price["USDC"], 100_000)
        self.assertEqual(idx.last_price_timestamp["USDC"], 1_000)

        idx.handle_event(["PriceUpdated", "USDC"], {"new_price": 101_000}, ledger_close_time=1_060)
        self.assertEqual(idx.last_price_timestamp["USDC"], 1_060)
        self.assertEqual(idx.price_updated_events_total["USDC"], 2)

    def test_deviation_flags_outlier_submission(self):
        idx = EventIndex("CTEST")
        idx.handle_event(["PriceUpdated", "XLM"], {"new_price": 100_000_000}, ledger_close_time=1_000)

        # Source submits a price 15% above the current aggregate.
        idx.handle_event(
            ["PriceSubmitted", "XLM", "rogue-source"],
            {"price": 115_000_000},
            ledger_close_time=1_010,
        )
        self.assertEqual(idx.source_deviation_bps[("XLM", "rogue-source")], 1500)

    def test_source_health_counters(self):
        idx = EventIndex("CTEST")
        idx.handle_event(["SourceAdded", "srcA"], {}, ledger_close_time=1_000)
        idx.handle_event(["SourceAdded", "srcB"], {}, ledger_close_time=1_000)
        self.assertEqual(idx.registered_sources_total, 2)
        self.assertEqual(idx.active_sources_total, 2)

        idx.handle_event(["SourceSuspended", "srcB"], {}, ledger_close_time=1_100)
        self.assertEqual(idx.registered_sources_total, 2)
        self.assertEqual(idx.active_sources_total, 1)

        idx.handle_event(["SourceRemoved", "srcA"], {}, ledger_close_time=1_200)
        self.assertEqual(idx.registered_sources_total, 1)

    def test_paused_gauge_toggles(self):
        idx = EventIndex("CTEST")
        self.assertEqual(idx.paused, 0)
        idx.handle_event(["ContractPaused"], {}, ledger_close_time=1_000)
        self.assertEqual(idx.paused, 1)
        idx.handle_event(["ContractUnpaused"], {}, ledger_close_time=1_100)
        self.assertEqual(idx.paused, 0)

    def test_render_emits_prometheus_text_format(self):
        idx = EventIndex("CTEST")
        idx.handle_event(["SourceAdded", "srcA"], {}, ledger_close_time=1_000)
        body = idx.render(now=2_000)
        self.assertIn('oracle_registered_sources_total{contract_id="CTEST"} 1', body)


if __name__ == "__main__":
    unittest.main()
