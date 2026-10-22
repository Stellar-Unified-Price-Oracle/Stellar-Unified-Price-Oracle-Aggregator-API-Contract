"""Synthetic incident evidence, assembled at runtime (#687).

The fixtures deliberately contain **no committed key-shaped string**. The
repository scans its own tree for Stellar seeds and generic credentials on
every commit (``.husky/pre-commit``, ``make secret-scan``), so a literal
``SXXXX…`` fixture would either fail the build or — worse — force an allowlist
entry for a "secret" that is not one. Building the values from parts keeps the
test adversarial without teaching the scanner to ignore a real shape.

Everything here is deterministic: same seed, same evidence, so a redaction
regression is reproducible.
"""

from __future__ import annotations

import base64
import random
from typing import List

BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"

#: A PEM header is assembled from parts for the same reason the seeds are: the
#: repository's own scanner flags a literal ``BEGIN PRIVATE KEY`` line, and an
#: allowlist entry for a fixture would train the scanner to ignore the shape.
PEM_BEGIN = "-----" + "BEGIN " + "PRIVATE" + " KEY-----"
PEM_END = "-----" + "END " + "PRIVATE" + " KEY-----"

#: The account whose (synthetic) seed appears in the evidence.
OPERATOR_ACCOUNT = "G" + "A" * 55
CONTRACT_ID = "C" + "C" * 55


def synthetic_seed(seed: int = 7) -> str:
    """A syntactically valid Stellar secret seed that is not a key."""
    rng = random.Random(seed)
    return "S" + "".join(rng.choice(BASE32) for _ in range(55))


def synthetic_jwt(seed: int = 11) -> str:
    rng = random.Random(seed)

    def segment(length: int) -> str:
        alphabet = BASE32 + "abcdefghijklmnopqrstuvwxyz0123456789-_"
        return "".join(rng.choice(alphabet) for _ in range(length))

    return "eyJ" + segment(18) + "." + segment(24) + "." + segment(20)


def synthetic_basic_auth(seed: int = 13) -> str:
    raw = f"operator:{seed}".encode()
    return "Basic " + base64.b64encode(raw).decode()


def synthetic_pem_body(seed: int = 17) -> str:
    """A distinctive fake PEM body, so the gate can prove it was removed."""
    raw = (f"synthetic-pem-{seed}" * 4).encode()
    encoded = base64.b64encode(raw).decode()
    return "\n".join(encoded[i : i + 64] for i in range(0, len(encoded), 64))


def synthetic_incident_log(seed: int = 7) -> str:
    """A realistic evidence bundle: CLI output, an RPC failure, an env dump."""
    stellar_seed = synthetic_seed(seed)
    jwt = synthetic_jwt(seed)
    basic = synthetic_basic_auth(seed)
    db_password = "hunter2-" + str(seed)
    return "\n".join(
        [
            "$ oracle-cli add-source --address " + OPERATOR_ACCOUNT + " --name chainlink"
            " --admin-key " + stellar_seed,
            "[e2e] submitted price for " + CONTRACT_ID,
            "curl: (22) 401 Unauthorized for https://soroban-testnet.stellar.org",
            "request headers: Authorization: " + basic,
            "retry used Authorization: Bearer " + jwt,
            "trace: token " + jwt,
            "export ORACLE_SOURCE_KEY=" + stellar_seed,
            "export ADMIN_SECRET_KEY=" + stellar_seed,
            "warn: unlabelled seed " + stellar_seed + " in a stack trace",
            '{"level":"error","api_key":"' + stellar_seed + '","msg":"rpc rejected"}',
            "postgres://oracle:" + db_password + "@db.internal:5432/oracle",
            "service-account key follows:",
            PEM_BEGIN,
            synthetic_pem_body(seed),
            PEM_END,
            "state dump: admin=" + OPERATOR_ACCOUNT + " decimals=18",
        ]
    )


def synthetic_secrets(seed: int = 7) -> List[str]:
    """Every value in `synthetic_incident_log` that must never survive."""
    jest = synthetic_jwt(seed)
    basic = synthetic_basic_auth(seed)
    # `Basic <b64>` is redacted as a whole, so the encoded credential is the
    # value that must disappear; the scheme word itself is kept on purpose.
    encoded = basic.split(" ", 1)[1]
    return [
        synthetic_seed(seed),
        jest,
        encoded,
        "hunter2-" + str(seed),
        synthetic_pem_body(seed).splitlines()[0],
    ]
