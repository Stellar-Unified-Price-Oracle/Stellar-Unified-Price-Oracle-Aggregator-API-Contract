"""Tests for OpenAPI schema validation and completeness."""
import unittest
import yaml
import os
from pathlib import Path


class TestOpenAPISchema(unittest.TestCase):
    """Tests for the OpenAPI specification."""

    @classmethod
    def setUpClass(cls):
        schema_path = Path(__file__).parent / "openapi.yaml"
        with open(schema_path) as f:
            cls.spec = yaml.safe_load(f)

    def test_openapi_version_present(self):
        self.assertIn("openapi", self.spec)
        self.assertEqual(self.spec["openapi"], "3.0.0")

    def test_info_section_present(self):
        self.assertIn("info", self.spec)
        info = self.spec["info"]
        self.assertIn("title", info)
        self.assertIn("version", info)
        self.assertIn("description", info)

    def test_info_has_title(self):
        self.assertEqual(self.spec["info"]["title"], "Stellar Unified Price Oracle API")

    def test_info_has_contact(self):
        self.assertIn("contact", self.spec["info"])
        contact = self.spec["info"]["contact"]
        self.assertIn("name", contact)
        self.assertIn("url", contact)

    def test_servers_defined(self):
        self.assertIn("servers", self.spec)
        self.assertGreater(len(self.spec["servers"]), 0)

    def test_testnet_server_configured(self):
        servers = self.spec["servers"]
        server_urls = [s["url"] for s in servers]
        self.assertIn("https://soroban-testnet.stellar.org", server_urls)

    def test_paths_section_present(self):
        self.assertIn("paths", self.spec)
        self.assertGreater(len(self.spec["paths"]), 0)

    def test_get_price_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/get_price", self.spec["paths"])

    def test_get_source_price_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/get_source_price", self.spec["paths"])

    def test_get_all_prices_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/get_all_prices", self.spec["paths"])

    def test_submit_price_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/submit_price", self.spec["paths"])

    def test_subscribe_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/subscribe", self.spec["paths"])

    def test_renew_subscription_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/renew_subscription", self.spec["paths"])

    def test_get_subscription_expiry_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/get_subscription_expiry", self.spec["paths"])

    def test_is_source_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/is_source", self.spec["paths"])

    def test_is_asset_registered_endpoint_exists(self):
        self.assertIn("/contracts/{contractId}/is_asset_registered", self.spec["paths"])

    def test_get_price_endpoint_has_method(self):
        path = self.spec["paths"]["/contracts/{contractId}/get_price"]
        self.assertIn("get", path)

    def test_submit_price_endpoint_has_method(self):
        path = self.spec["paths"]["/contracts/{contractId}/submit_price"]
        self.assertIn("post", path)

    def test_subscribe_endpoint_has_method(self):
        path = self.spec["paths"]["/contracts/{contractId}/subscribe"]
        self.assertIn("post", path)

    def test_get_price_has_operation_id(self):
        op = self.spec["paths"]["/contracts/{contractId}/get_price"]["get"]
        self.assertIn("operationId", op)
        self.assertEqual(op["operationId"], "getPrice")

    def test_submit_price_has_operation_id(self):
        op = self.spec["paths"]["/contracts/{contractId}/submit_price"]["post"]
        self.assertIn("operationId", op)
        self.assertEqual(op["operationId"], "submitPrice")

    def test_get_price_has_parameters(self):
        op = self.spec["paths"]["/contracts/{contractId}/get_price"]["get"]
        self.assertIn("parameters", op)
        param_names = [p["name"] for p in op["parameters"]]
        self.assertIn("asset", param_names)
        self.assertIn("maxAge", param_names)

    def test_get_price_has_responses(self):
        op = self.spec["paths"]["/contracts/{contractId}/get_price"]["get"]
        self.assertIn("responses", op)
        self.assertIn("200", op["responses"])

    def test_components_section_present(self):
        self.assertIn("components", self.spec)

    def test_schemas_defined(self):
        self.assertIn("schemas", self.spec["components"])

    def test_price_entry_schema_exists(self):
        schemas = self.spec["components"]["schemas"]
        self.assertIn("PriceEntry", schemas)

    def test_price_entry_has_required_fields(self):
        schema = self.spec["components"]["schemas"]["PriceEntry"]
        self.assertIn("required", schema)
        self.assertIn("price", schema["required"])
        self.assertIn("timestamp", schema["required"])

    def test_price_entry_properties(self):
        schema = self.spec["components"]["schemas"]["PriceEntry"]
        self.assertIn("properties", schema)
        props = schema["properties"]
        self.assertIn("price", props)
        self.assertIn("timestamp", props)

    def test_submit_price_request_schema_exists(self):
        schemas = self.spec["components"]["schemas"]
        self.assertIn("SubmitPriceRequest", schemas)

    def test_submit_price_request_required_fields(self):
        schema = self.spec["components"]["schemas"]["SubmitPriceRequest"]
        self.assertIn("required", schema)
        required = schema["required"]
        self.assertIn("source", required)
        self.assertIn("asset", required)
        self.assertIn("price", required)
        self.assertIn("timestamp", required)

    def test_subscribe_request_schema_exists(self):
        schemas = self.spec["components"]["schemas"]
        self.assertIn("SubscribeRequest", schemas)

    def test_renew_subscription_request_schema_exists(self):
        schemas = self.spec["components"]["schemas"]
        self.assertIn("RenewSubscriptionRequest", schemas)

    def test_transaction_result_schema_exists(self):
        schemas = self.spec["components"]["schemas"]
        self.assertIn("TransactionResult", schemas)

    def test_transaction_result_required_fields(self):
        schema = self.spec["components"]["schemas"]["TransactionResult"]
        self.assertIn("required", schema)
        self.assertIn("hash", schema["required"])
        self.assertIn("status", schema["required"])

    def test_security_schemes_defined(self):
        self.assertIn("securitySchemes", self.spec["components"])

    def test_stellar_signature_security_scheme(self):
        schemes = self.spec["components"]["securitySchemes"]
        self.assertIn("stellarSignature", schemes)

    def test_submit_price_requires_auth(self):
        op = self.spec["paths"]["/contracts/{contractId}/submit_price"]["post"]
        self.assertIn("security", op)

    def test_subscribe_requires_auth(self):
        op = self.spec["paths"]["/contracts/{contractId}/subscribe"]["post"]
        self.assertIn("security", op)

    def test_all_endpoints_have_responses(self):
        for path_name, path_item in self.spec["paths"].items():
            for method_name, operation in path_item.items():
                if method_name in ["get", "post", "put", "delete"]:
                    self.assertIn(
                        "responses",
                        operation,
                        f"{method_name.upper()} {path_name} missing responses",
                    )

    def test_all_endpoints_have_descriptions(self):
        for path_name, path_item in self.spec["paths"].items():
            for method_name, operation in path_item.items():
                if method_name in ["get", "post", "put", "delete"]:
                    self.assertIn(
                        "description",
                        operation,
                        f"{method_name.upper()} {path_name} missing description",
                    )

    def test_error_codes_documented(self):
        op = self.spec["paths"]["/contracts/{contractId}/get_price"]["get"]
        responses = op["responses"]
        self.assertIn("400", responses)
        self.assertIn("404", responses)

    def test_contract_id_parameter_in_all_paths(self):
        for path_name in self.spec["paths"].keys():
            self.assertIn("contractId", path_name)

    def test_reference_types_used(self):
        op = self.spec["paths"]["/contracts/{contractId}/get_price"]["get"]
        response = op["responses"]["200"]
        self.assertIn("content", response)
        self.assertIn("$ref", response["content"]["application/json"]["schema"])


class TestOpenAPISchemaFile(unittest.TestCase):
    """Tests for OpenAPI schema file properties."""

    def test_openapi_file_exists(self):
        schema_path = Path(__file__).parent / "openapi.yaml"
        self.assertTrue(schema_path.exists(), "openapi.yaml file does not exist")

    def test_openapi_file_is_valid_yaml(self):
        schema_path = Path(__file__).parent / "openapi.yaml"
        try:
            with open(schema_path) as f:
                yaml.safe_load(f)
        except yaml.YAMLError as e:
            self.fail(f"openapi.yaml is not valid YAML: {e}")

    def test_openapi_file_not_empty(self):
        schema_path = Path(__file__).parent / "openapi.yaml"
        with open(schema_path) as f:
            content = f.read()
        self.assertGreater(len(content), 0)


if __name__ == "__main__":
    unittest.main()
