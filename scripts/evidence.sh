#!/usr/bin/env bash
# evidence.sh — collect an incident evidence bundle with credentials redacted (#687).
#
# Usage:
#   ./scripts/evidence.sh                       # print to stdout
#   ./scripts/evidence.sh --out incident.md     # write redacted artifact + report
#   ./scripts/evidence.sh --out incident.md app.log rpc.log
#
# Everything captured here — environment, contract state, log tails — is piped
# through services.evidence_redaction.redact *before* it is written. The
# redactor has no pass-through mode, so if it is unavailable this script fails
# and produces no artifact at all. That is deliberate: a partially redacted
# artifact is worse than no artifact, because it looks safe.
#
# Public chain data (account addresses, contract ids) is kept; secret seeds,
# Authorization headers, credential flags, env values and URL passwords are not.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
OUT=""
FILES=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --out) OUT="$2"; shift 2 ;;
        -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) FILES+=("$1"); shift ;;
    esac
done

RAW="$(mktemp)"
trap 'rm -f "$RAW"' EXIT

{
    echo "# Incident evidence"
    echo
    echo "- collected_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- commit: $(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    echo "- network: ${ORACLE_NETWORK:-unset}"
    echo "- contract: ${ORACLE_CONTRACT_ID:-unset}"
    echo
    echo "## Environment (credential values redacted)"
    # Names and values both matter for triage; the redactor removes the values
    # that are credentials and leaves the rest.
    env | grep -E '^(ORACLE|STELLAR|ADMIN|RPC|SOROBAN|AWS|GITHUB|DATABASE|POSTGRES)' | sort || true
    echo
    echo "## Contract state"
    if [[ -n "${ORACLE_CONTRACT_ID:-}" ]]; then
        "$SCRIPT_DIR/oracle-cli.sh" oracle-state-dump 2>&1 || echo "state dump failed"
    else
        echo "ORACLE_CONTRACT_ID is not set; state dump skipped"
    fi
    echo
    echo "## Recent logs"
    if [[ ${#FILES[@]} -eq 0 ]]; then
        echo "no log files supplied"
    fi
    for path in "${FILES[@]:-}"; do
        [[ -n "$path" ]] || continue
        echo "### $path"
        tail -n 2000 "$path" 2>&1 || echo "unreadable: $path"
        echo
    done
} > "$RAW"

if [[ -n "$OUT" ]]; then
    PYTHONPATH="$REPO_ROOT" python3 -m services.evidence_redaction.redact "$RAW" \
        --out "$OUT" \
        --report "${OUT%.*}.redaction.md"
    echo "evidence written to $OUT (report: ${OUT%.*}.redaction.md)"
else
    PYTHONPATH="$REPO_ROOT" python3 -m services.evidence_redaction.redact "$RAW"
fi
