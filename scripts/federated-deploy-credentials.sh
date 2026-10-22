#!/usr/bin/env bash
# federated-deploy-credentials.sh — exchange a GitHub Actions OIDC token for
# short-lived deploy credentials (#524).
#
# Usage:
#   eval "$(./scripts/federated-deploy-credentials.sh echo-env --role <ROLE_ARN>)"
#   eval "$(./scripts/federated-deploy-credentials.sh write-key --key-name <NAME>)"
#
# Subcommands:
#   echo-env   Exchange the OIDC token for cloud credentials and print
#              `export` lines for them. Nothing long-lived is written anywhere.
#   write-key  Additionally fetch the short-lived deploy signer (a Stellar
#              secret key) from the provider's secret store and write it to
#              $FEDERATED_KEY_FILE with mode 0600, printing the path.
#
# The federation trust policy that authorises ROLE_ARN is pinned in
# services/ci_policy/policy.py and audited by `make ci-audit`; see
# docs/secretless-ci.md. Only the *identifier* of the role is passed in — the
# role itself lives in the cloud account, not in this repository.
#
# Requires the workflow to request an ID token:
#   permissions:
#     id-token: write
#
# Environment (provided automatically by GitHub Actions):
#   ACTIONS_ID_TOKEN_REQUEST_URL, ACTIONS_ID_TOKEN_REQUEST_TOKEN
#
# Break-glass: if federation is unavailable, use the documented manual path in
# docs/secretless-ci.md §5. This script has no static-credential fallback on
# purpose — a fallback would reintroduce exactly the secret being removed.

set -euo pipefail

FEDERATED_KEY_FILE="${FEDERATED_KEY_FILE:-${RUNNER_TEMP:-/tmp}/federated-deploy-key}"
KEY_VALIDITY_SECONDS="${KEY_VALIDITY_SECONDS:-900}"

die() { echo "federated-deploy-credentials: $*" >&2; exit 1; }

# Fetches a GitHub Actions OIDC token for the given audience. The token is
# single-use, short-lived (minutes), and scoped by the provider's trust policy
# to this repository, workflow file, ref and environment.
fetch_oidc_token() {
  local audience="$1"
  command -v curl >/dev/null || die "curl is required"
  [ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" ] || die \
    "ACTIONS_ID_TOKEN_REQUEST_URL is unset — the job needs 'permissions: id-token: write'"
  [ -n "${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-}" ] || die \
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN is unset — the job needs 'permissions: id-token: write'"
  curl -fsSL -H "Authorization: bearer ${ACTIONS_ID_TOKEN_REQUEST_TOKEN}" \
    "${ACTIONS_ID_TOKEN_REQUEST_URL}&audience=${audience}"
}

assume_aws_role() {
  local role_arn="$1"
  command -v aws >/dev/null || die "aws CLI is required for AWS federation"
  # --web-identity-token-file: the token comes from the OIDC exchange above,
  # so the resulting session is scoped to the trust policy, not to a stored key.
  aws sts assume-role-with-web-identity \
    --role-arn "$role_arn" \
    --role-session-name "gha-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT" \
    --web-identity-token "$(fetch_oidc_token sts.amazonaws.com | tr -d '\n' | sed -n 's/.*"id_token":"\([^"]*\)".*/\1/p')" \
    --duration-seconds 3600 \
    --query 'Credentials.[AccessKeyId,SecretAccessKey,SessionToken]' \
    --output text
}

assume_gcp_workload() {
  local provider="$1"
  command -v gcloud >/dev/null || die "gcloud CLI is required for GCP federation"
  local token
  token="$(fetch_oidc_token "$provider" | tr -d '\n' | sed -n 's/.*"id_token":"\([^"]*\)".*/\1/p')"
  [ -n "$token" ] || die "could not extract id_token from the OIDC response"
  echo "$token" | gcloud iam workload-identity-pools create-cred-config "$provider" \
    --service-account="$GCP_DEPLOY_SERVICE_ACCOUNT" \
    --output-file="$FEDERATED_KEY_FILE.cred.json" >/dev/null
  gcloud auth activate-service-account --cred-file="$FEDERATED_KEY_FILE.cred.json" >/dev/null
  rm -f "$FEDERATED_KEY_FILE.cred.json"
}

# Reads the short-lived deploy signer. It is fetched with the federated
# session, so it is never a repository secret and never at rest in the repo.
fetch_deploy_signer() {
  local key_name="$1"
  if command -v aws >/dev/null && [ -n "${AWS_SECRET_NAME:-}" ]; then
    aws secretsmanager get-secret-value --secret-id "$key_name" \
      --query SecretString --output text
  elif command -v gcloud >/dev/null && [ -n "${GCP_DEPLOY_SERVICE_ACCOUNT:-}" ]; then
    gcloud secrets versions access latest --secret="$key_name" --format='value(payload.data)'
  else
    die "no provider available to fetch '$key_name' (set AWS_SECRET_NAME or GCP_DEPLOY_SERVICE_ACCOUNT)"
  fi
}

subcommand="${1:-}"
[ -n "$subcommand" ] || die "usage: $0 {echo-env|write-key} --role <ROLE_ARN> [--key-name <NAME>]"
shift

role=""
key_name=""
while [ $# -gt 0 ]; do
  case "$1" in
    --role)      role="$2"; shift 2 ;;
    --key-name)  key_name="$2"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

case "$role" in
  arn:aws:*) assume_aws_role "$role" | {
      read -r access secret session
      [ -n "$session" ] || die "sts did not return a session token"
      echo "export AWS_ACCESS_KEY_ID=$access"
      echo "export AWS_SECRET_ACCESS_KEY=$secret"
      echo "export AWS_SESSION_TOKEN=$session"
    } ;;
  //iam.googleapis.com/*) assume_gcp_workload "$role" ;;
  *) die "--role must be an AWS IAM role ARN or a GCP workload identity provider" ;;
esac

# Audit trail: the exchange is attributable to this commit and run. The
# federation token is ephemeral, so this record is the durable one.
echo "federated credentials assumed for role=$role run=$GITHUB_RUN_ID/$GITHUB_RUN_ATTEMPT commit=$GITHUB_SHA" >&2

if [ "$subcommand" = "write-key" ]; then
  [ -n "$key_name" ] || die "write-key requires --key-name"
  fetch_deploy_signer "$key_name" > "$FEDERATED_KEY_FILE"
  chmod 600 "$FEDERATED_KEY_FILE"
  echo "FEDERATED_KEY_FILE=$FEDERATED_KEY_FILE"
elif [ "$subcommand" != "echo-env" ]; then
  die "unknown subcommand: $subcommand"
fi
