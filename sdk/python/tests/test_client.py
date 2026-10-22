"""Tests for the Oracle SDK client."""
import unittest
from unittest.mock import Mock, MagicMock, patch
from dataclasses import dataclass

from oracle_sdk.client import OracleClient, OracleClientConfig, PriceEntry


class TestPriceEntry(unittest.TestCase):
    """Tests for PriceEntry data class."""

    def test_price_entry_creation(self):
        entry = PriceEntry(price=1000000, timestamp=1234567890)
        self.assertEqual(entry.price, 1000000)
        self.assertEqual(entry.timestamp, 1234567890)

    def test_price_entry_with_different_values(self):
        entry = PriceEntry(price=999999999, timestamp=9999999999)
        self.assertEqual(entry.price, 999999999)
        self.assertEqual(entry.timestamp, 9999999999)


class TestOracleClientConfig(unittest.TestCase):
    """Tests for OracleClientConfig data class."""

    def test_config_creation_with_required_params(self):
        config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )
        self.assertEqual(config.contract_id, "CABC123DEF456")
        self.assertEqual(config.rpc_url, "https://soroban-testnet.stellar.org")

    def test_config_creation_with_custom_network(self):
        network = "Test SDF Network ; September 2015"
        config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
            network_passphrase=network,
        )
        self.assertEqual(config.network_passphrase, network)

    def test_config_has_testnet_default_passphrase(self):
        config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )
        self.assertIn("Testnet", config.network_passphrase)


class TestOracleClientInitialization(unittest.TestCase):
    """Tests for OracleClient initialization."""

    def test_client_initialization(self):
        config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )
        with patch("oracle_sdk.client.SorobanServer"):
            client = OracleClient(config)
            self.assertEqual(client.config, config)
            self.assertIsNotNone(client.server)

    def test_client_config_stored(self):
        config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )
        with patch("oracle_sdk.client.SorobanServer"):
            client = OracleClient(config)
            self.assertEqual(client.config.contract_id, "CABC123DEF456")
            self.assertEqual(client.config.rpc_url, "https://soroban-testnet.stellar.org")


class TestOracleClientViewMethods(unittest.TestCase):
    """Tests for OracleClient view (read-only) methods."""

    def setUp(self):
        self.config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )

    @patch("oracle_sdk.client.SorobanServer")
    def test_get_price_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "get_price"))
        self.assertTrue(callable(getattr(client, "get_price")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_get_source_price_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "get_source_price"))
        self.assertTrue(callable(getattr(client, "get_source_price")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_get_all_prices_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "get_all_prices"))
        self.assertTrue(callable(getattr(client, "get_all_prices")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_get_subscription_expiry_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "get_subscription_expiry"))
        self.assertTrue(callable(getattr(client, "get_subscription_expiry")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_is_source_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "is_source"))
        self.assertTrue(callable(getattr(client, "is_source")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_is_asset_registered_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "is_asset_registered"))
        self.assertTrue(callable(getattr(client, "is_asset_registered")))


class TestOracleClientInvokeMethods(unittest.TestCase):
    """Tests for OracleClient invoke (transaction) methods."""

    def setUp(self):
        self.config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )

    @patch("oracle_sdk.client.SorobanServer")
    def test_submit_price_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "submit_price"))
        self.assertTrue(callable(getattr(client, "submit_price")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_subscribe_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "subscribe"))
        self.assertTrue(callable(getattr(client, "subscribe")))

    @patch("oracle_sdk.client.SorobanServer")
    def test_renew_subscription_method_exists(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "renew_subscription"))
        self.assertTrue(callable(getattr(client, "renew_subscription")))


class TestOracleClientMethodSignatures(unittest.TestCase):
    """Tests for OracleClient method signatures and parameter types."""

    def setUp(self):
        self.config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )

    @patch("oracle_sdk.client.SorobanServer")
    def test_get_price_accepts_correct_params(self, mock_server):
        client = OracleClient(self.config)
        method = getattr(client, "get_price")
        import inspect
        sig = inspect.signature(method)
        params = list(sig.parameters.keys())
        self.assertIn("asset", params)
        self.assertIn("max_age", params)

    @patch("oracle_sdk.client.SorobanServer")
    def test_submit_price_accepts_correct_params(self, mock_server):
        client = OracleClient(self.config)
        method = getattr(client, "submit_price")
        import inspect
        sig = inspect.signature(method)
        params = list(sig.parameters.keys())
        self.assertIn("source", params)
        self.assertIn("asset", params)
        self.assertIn("price", params)
        self.assertIn("timestamp", params)
        self.assertIn("signer", params)

    @patch("oracle_sdk.client.SorobanServer")
    def test_subscribe_accepts_correct_params(self, mock_server):
        client = OracleClient(self.config)
        method = getattr(client, "subscribe")
        import inspect
        sig = inspect.signature(method)
        params = list(sig.parameters.keys())
        self.assertIn("consumer", params)
        self.assertIn("duration", params)
        self.assertIn("signer", params)


class TestOracleClientIntegration(unittest.TestCase):
    """Integration tests for OracleClient."""

    def setUp(self):
        self.config = OracleClientConfig(
            contract_id="CABC123DEF456",
            rpc_url="https://soroban-testnet.stellar.org",
        )

    @patch("oracle_sdk.client.SorobanServer")
    def test_client_has_all_methods(self, mock_server):
        client = OracleClient(self.config)
        required_methods = [
            "get_price",
            "get_source_price",
            "get_all_prices",
            "submit_price",
            "subscribe",
            "renew_subscription",
            "get_subscription_expiry",
            "is_source",
            "is_asset_registered",
        ]
        for method_name in required_methods:
            self.assertTrue(
                hasattr(client, method_name),
                f"Client missing method: {method_name}",
            )

    @patch("oracle_sdk.client.SorobanServer")
    def test_client_internal_methods(self, mock_server):
        client = OracleClient(self.config)
        self.assertTrue(hasattr(client, "_invoke"))
        self.assertTrue(hasattr(client, "_view"))


if __name__ == "__main__":
    unittest.main()
