"""Tests for GraphQL schema and query validation."""
import unittest
from pathlib import Path


class TestGraphQLSchema(unittest.TestCase):
    """Tests for the GraphQL schema structure."""

    @classmethod
    def setUpClass(cls):
        schema_path = Path(__file__).parent / "schema.graphql"
        with open(schema_path) as f:
            cls.schema_content = f.read()

    def test_schema_file_exists(self):
        schema_path = Path(__file__).parent / "schema.graphql"
        self.assertTrue(schema_path.exists(), "schema.graphql file does not exist")

    def test_schema_file_not_empty(self):
        self.assertGreater(len(self.schema_content), 0, "schema.graphql is empty")

    def test_query_type_defined(self):
        self.assertIn("type Query", self.schema_content)

    def test_mutation_type_defined(self):
        self.assertIn("type Mutation", self.schema_content)

    def test_subscription_type_defined(self):
        self.assertIn("type Subscription", self.schema_content)

    def test_prices_query_defined(self):
        self.assertIn("prices(", self.schema_content)
        self.assertIn("asset: String!", self.schema_content)

    def test_latest_price_query_defined(self):
        self.assertIn("latestPrice(", self.schema_content)

    def test_price_stats_query_defined(self):
        self.assertIn("priceStats(", self.schema_content)

    def test_sources_query_defined(self):
        self.assertIn("sources:", self.schema_content)

    def test_source_stats_query_defined(self):
        self.assertIn("sourceStats(", self.schema_content)

    def test_aggregated_prices_query_defined(self):
        self.assertIn("aggregatedPrices(", self.schema_content)

    def test_submissions_by_source_query_defined(self):
        self.assertIn("submissionsBySource(", self.schema_content)

    def test_events_query_defined(self):
        self.assertIn("events(", self.schema_content)

    def test_search_prices_query_defined(self):
        self.assertIn("searchPrices(", self.schema_content)

    def test_price_data_point_type_defined(self):
        self.assertIn("type PriceDataPoint", self.schema_content)

    def test_price_statistics_type_defined(self):
        self.assertIn("type PriceStatistics", self.schema_content)

    def test_source_type_defined(self):
        self.assertIn("type Source", self.schema_content)

    def test_source_statistics_type_defined(self):
        self.assertIn("type SourceStatistics", self.schema_content)

    def test_aggregated_price_type_defined(self):
        self.assertIn("type AggregatedPrice", self.schema_content)

    def test_price_submission_type_defined(self):
        self.assertIn("type PriceSubmission", self.schema_content)

    def test_price_event_type_defined(self):
        self.assertIn("type PriceEvent", self.schema_content)

    def test_aggregation_method_enum_defined(self):
        self.assertIn("enum AggregationMethod", self.schema_content)

    def test_event_type_enum_defined(self):
        self.assertIn("enum EventType", self.schema_content)

    def test_price_filter_input_defined(self):
        self.assertIn("input PriceFilter", self.schema_content)

    def test_pagination_input_defined(self):
        self.assertIn("input PaginationInput", self.schema_content)

    def test_time_range_type_defined(self):
        self.assertIn("type TimeRange", self.schema_content)

    def test_price_search_result_type_defined(self):
        self.assertIn("type PriceSearchResult", self.schema_content)

    def test_subscribe_to_prices_mutation_defined(self):
        self.assertIn("subscribeToPrices(", self.schema_content)

    def test_unsubscribe_mutation_defined(self):
        self.assertIn("unsubscribeToPrices(", self.schema_content)

    def test_price_updated_subscription_defined(self):
        self.assertIn("priceUpdated(", self.schema_content)

    def test_price_aggregated_subscription_defined(self):
        self.assertIn("priceAggregated:", self.schema_content)

    def test_price_data_point_has_asset_field(self):
        self.assertIn("asset: String!", self.schema_content)

    def test_price_data_point_has_price_field(self):
        self.assertIn("price: String!", self.schema_content)

    def test_price_data_point_has_timestamp_field(self):
        self.assertIn("timestamp: Int!", self.schema_content)

    def test_source_has_address_field(self):
        # Check that Source type has address field
        self.assertIn("address: String!", self.schema_content)

    def test_source_has_is_active_field(self):
        self.assertIn("isActive: Boolean!", self.schema_content)

    def test_aggregation_methods_available(self):
        self.assertIn("MEDIAN", self.schema_content)
        self.assertIn("MEAN", self.schema_content)
        self.assertIn("WEIGHTED_MEAN", self.schema_content)

    def test_event_types_available(self):
        self.assertIn("PRICE_SUBMITTED", self.schema_content)
        self.assertIn("PRICE_AGGREGATED", self.schema_content)
        self.assertIn("SOURCE_REGISTERED", self.schema_content)

    def test_prices_query_has_time_range_params(self):
        self.assertIn("fromTimestamp: Int!", self.schema_content)
        self.assertIn("toTimestamp: Int!", self.schema_content)

    def test_prices_query_has_resolution_param(self):
        self.assertIn("resolution: Int", self.schema_content)

    def test_aggregated_prices_has_method_param(self):
        self.assertIn("method: AggregationMethod", self.schema_content)

    def test_search_prices_returns_paginated_result(self):
        self.assertIn("PriceSearchResult", self.schema_content)
        self.assertIn("hasMore: Boolean!", self.schema_content)

    def test_source_statistics_has_uptime_metric(self):
        self.assertIn("uptime: Float!", self.schema_content)

    def test_source_statistics_has_reliability_metric(self):
        self.assertIn("reliability: Float!", self.schema_content)

    def test_price_statistics_has_median(self):
        self.assertIn("median: String!", self.schema_content)

    def test_price_statistics_has_std_deviation(self):
        self.assertIn("stdDeviation: String!", self.schema_content)

    def test_aggregated_price_has_confidence(self):
        self.assertIn("confidence: Float!", self.schema_content)

    def test_price_submission_has_success_flag(self):
        self.assertIn("success: Boolean!", self.schema_content)

    def test_query_documentation_present(self):
        # Check for at least one documented field
        self.assertIn('"""', self.schema_content)

    def test_pagination_has_limit_parameter(self):
        self.assertIn("limit: Int", self.schema_content)

    def test_pagination_has_offset_parameter(self):
        self.assertIn("offset: Int", self.schema_content)

    def test_pagination_has_cursor_parameter(self):
        self.assertIn("cursor: String", self.schema_content)


class TestGraphQLQueries(unittest.TestCase):
    """Tests for GraphQL query structure and requirements."""

    @classmethod
    def setUpClass(cls):
        schema_path = Path(__file__).parent / "schema.graphql"
        with open(schema_path) as f:
            cls.schema_content = f.read()

    def test_prices_query_returns_array(self):
        self.assertIn("prices(", self.schema_content)
        self.assertIn(": [PriceDataPoint!]!", self.schema_content)

    def test_sources_query_returns_array(self):
        self.assertIn("sources:", self.schema_content)
        self.assertIn(": [Source!]!", self.schema_content)

    def test_events_query_supports_filtering(self):
        # Check for event filtering parameters
        self.assertIn("eventType: EventType", self.schema_content)

    def test_submissions_by_source_has_limit(self):
        self.assertIn("submissionsBySource(", self.schema_content)
        self.assertIn("limit: Int", self.schema_content)

    def test_search_prices_uses_filter_input(self):
        self.assertIn("filter: PriceFilter!", self.schema_content)

    def test_search_prices_uses_pagination_input(self):
        self.assertIn("pagination: PaginationInput!", self.schema_content)


class TestGraphQLTypes(unittest.TestCase):
    """Tests for GraphQL type definitions and fields."""

    @classmethod
    def setUpClass(cls):
        schema_path = Path(__file__).parent / "schema.graphql"
        with open(schema_path) as f:
            cls.schema_content = f.read()

    def test_price_data_point_all_fields(self):
        point_section = self.schema_content[
            self.schema_content.find("type PriceDataPoint") :
            self.schema_content.find("type PriceDataPoint") + 500
        ]
        self.assertIn("asset:", point_section)
        self.assertIn("price:", point_section)
        self.assertIn("timestamp:", point_section)

    def test_price_statistics_all_fields(self):
        stats_section = self.schema_content[
            self.schema_content.find("type PriceStatistics") :
            self.schema_content.find("type PriceStatistics") + 600
        ]
        self.assertIn("average:", stats_section)
        self.assertIn("median:", stats_section)
        self.assertIn("minimum:", stats_section)

    def test_source_statistics_all_fields(self):
        stats_section = self.schema_content[
            self.schema_content.find("type SourceStatistics") :
            self.schema_content.find("type SourceStatistics") + 700
        ]
        self.assertIn("totalSubmissions:", stats_section)
        self.assertIn("uptime:", stats_section)
        self.assertIn("reliability:", stats_section)

    def test_aggregated_price_has_sources_count(self):
        self.assertIn("sources: Int!", self.schema_content)

    def test_enum_values_properly_formatted(self):
        # Check for enum value formatting (UPPERCASE)
        self.assertIn("MEDIAN", self.schema_content)
        self.assertIn("PRICE_SUBMITTED", self.schema_content)

    def test_input_types_properly_formatted(self):
        self.assertIn("input PriceFilter", self.schema_content)
        self.assertIn("input PaginationInput", self.schema_content)


class TestGraphQLSubscriptions(unittest.TestCase):
    """Tests for GraphQL subscription support."""

    @classmethod
    def setUpClass(cls):
        schema_path = Path(__file__).parent / "schema.graphql"
        with open(schema_path) as f:
            cls.schema_content = f.read()

    def test_real_time_price_updates_subscription(self):
        self.assertIn("priceUpdated(", self.schema_content)
        self.assertIn(": PriceDataPoint!", self.schema_content)

    def test_aggregation_events_subscription(self):
        self.assertIn("priceAggregated:", self.schema_content)

    def test_subscribe_mutation_creates_subscription(self):
        self.assertIn("subscribeToPrices(", self.schema_content)
        self.assertIn(": Subscription!", self.schema_content)

    def test_unsubscribe_mutation_removes_subscription(self):
        self.assertIn("unsubscribeToPrices(", self.schema_content)
        self.assertIn(": Boolean!", self.schema_content)


if __name__ == "__main__":
    unittest.main()
