"""Secretless CI/CD: OIDC federation and secret-hygiene policy (#524).

Deploy credentials are the highest-value secret in the repo and the one most
often leaked. This module makes the migration to short-lived, OIDC-federated
credentials *checkable* rather than aspirational:

* :class:`TrustPolicy` encodes the OIDC trust relationship as a set of exact
  subject-claim matchers. :func:`evaluate` answers "may this token assume the
  deploy role?" — the question an auditor asks, answered by code and asserted
  by tests rather than by reading a wiki page.
* :func:`scan_workflows` / :func:`find_long_lived_secrets` prove the negative
  claim: no long-lived deploy credential is referenced anywhere in the
  repository's CI configuration or scripts.
* :class:`DeploymentAttestation` binds a deployment to a commit and a workflow
  run, so every deployment is attributable after the fact.

Provider-neutral on purpose: the trust semantics (repository, ref, environment,
workflow identity) are GitHub's, and are what a federated role must be scoped
on. The concrete cloud provider only supplies the token exchange.

See ``docs/secretless-ci.md``.
"""
