#!/usr/bin/env python3
"""Chainlink-style external adapter reference implementation.

Exposes POST /submit accepting a standard Chainlink EA job-run payload,
verifies an HMAC-SHA256 request signature, and forwards the price to the
oracle contract via `stellar contract invoke ... submit_price`.

See docs/external-adapter.md for the request/response spec.
"""
import argparse
import hashlib
import hmac
import json
import os
import subprocess
from http.server import BaseHTTPRequestHandler, HTTPServer

SHARED_SECRET = os.environ.get("ADAPTER_SHARED_SECRET", "")


def verify_signature(body: bytes, signature: str) -> bool:
    if not SHARED_SECRET or not signature:
        return False
    expected = hmac.new(SHARED_SECRET.encode(), body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, signature)


def submit_price(contract_id: str, source_identity: str, network: str,
                  asset: str, price: str, timestamp: int) -> str:
    result = subprocess.run(
        [
            "stellar", "contract", "invoke",
            "--id", contract_id,
            "--source", source_identity,
            "--network", network,
            "--", "submit_price",
            "--source", source_identity,
            "--asset", asset,
            "--price", price,
            "--timestamp", str(timestamp),
        ],
        capture_output=True, text=True, check=True,
    )
    return result.stdout.strip()


def make_handler(contract_id: str, source_identity: str, network: str):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            if self.path != "/submit":
                self.send_response(404)
                self.end_headers()
                return

            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length)
            signature = self.headers.get("X-EA-Signature", "")
            job_id = None

            try:
                payload = json.loads(body)
                job_id = payload.get("id")

                if not verify_signature(body, signature):
                    raise PermissionError("invalid or missing signature")

                data = payload["data"]
                tx_hash = submit_price(
                    contract_id, source_identity, network,
                    data["asset"], str(data["price"]), int(data["timestamp"]),
                )
                response = {
                    "jobRunID": job_id,
                    "status": "success",
                    "data": {"result": str(data["price"]), "txHash": tx_hash},
                }
                status_code = 200
            except Exception as exc:  # noqa: BLE001 - EA envelope reports all failures uniformly
                response = {
                    "jobRunID": job_id,
                    "status": "errored",
                    "data": {"error": str(exc)},
                }
                status_code = 400

            body_out = json.dumps(response).encode()
            self.send_response(status_code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body_out)))
            self.end_headers()
            self.wfile.write(body_out)

        def log_message(self, fmt, *args):
            pass

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", required=True, help="Oracle contract ID")
    parser.add_argument("--source-identity", required=True, help="Stellar CLI identity used to sign submissions")
    parser.add_argument("--network", default="testnet")
    parser.add_argument("--port", type=int, default=8080)
    args = parser.parse_args()

    if not SHARED_SECRET:
        raise SystemExit("ADAPTER_SHARED_SECRET environment variable must be set")

    handler = make_handler(args.contract, args.source_identity, args.network)
    server = HTTPServer(("0.0.0.0", args.port), handler)
    print(f"Chainlink adapter listening on :{args.port}")
    server.serve_forever()


if __name__ == "__main__":
    main()
