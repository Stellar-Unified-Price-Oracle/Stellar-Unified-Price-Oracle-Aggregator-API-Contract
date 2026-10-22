"""Signed release artifacts and SLSA provenance (#525).

Independent verification that the deployed contract matches the audited source
is a core assurance for an oracle. This module produces and checks the two
artifacts that make that verification automatable:

* a **provenance attestation** (in-toto Statement v1, SLSA v1.0 predicate)
  binding the WASM digest to the source commit, the workflow, the builder and
  the toolchain that produced it; and
* the **verification** of that attestation against a freshly built artifact,
  so CI can prove the published bytes match the source.

Signing is keyless: the signature's identity is the OIDC-federated workload
(``https://github.com/<repo>/.github/workflows/release.yml@refs/tags/<tag>``),
so there is no long-lived signing key to store, rotate, or leak. Signature and
transparency-log verification is delegated to ``cosign`` / ``gh attestation``;
this module owns the *content* of the attestation and the checks that do not
need a network.

See ``docs/release-provenance.md``.
"""
