#!/usr/bin/env bash
# Supply-chain malware scan (PolinRider and similar repo-poisoning campaigns).
#
# Fails if any tracked file shows a known sign of an injected payload:
#   - known PolinRider markers
#   - editor/devcontainer files that auto-run code when the folder is opened
#   - npm lifecycle hooks (preinstall/install/postinstall) in any package.json
#   - "font" or other asset files that actually contain JavaScript
#   - lines padded with hundreds of whitespace characters (payload hiding)
#   - obfuscated-JavaScript signatures
#
# Reads files only; never executes anything from the repo.
# Usage: scripts/supply-chain-scan.sh   (run from anywhere inside the repo)
# See docs/security/malware-incident-2026-10.md.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)"
SELF="scripts/supply-chain-scan.sh"
# Files that must quote the patterns to document them.
ALLOW=("$SELF" "docs/security/malware-incident-2026-10.md")
fail=0
report() { echo "::error::supply-chain-scan: $1"; fail=1; }

# Tracked files, minus the allowlist above.
mapfile -d '' FILES < <(git ls-files -z | grep -zvxF "${ALLOW[@]/#/-e}")

# Text-only subset for content checks (skips binaries).
text_files() { printf '%s\0' "${FILES[@]}" | xargs -0 grep -lIZ '' 2>/dev/null; }

# 1. Known campaign markers (split so this file doesn't match itself).
markers=("rmcej%""otb%" "Cot%3t""=shtP" '_$_''1e42' "global['""!']")
for m in "${markers[@]}"; do
  hits=$(printf '%s\0' "${FILES[@]}" | xargs -0 grep -laF -e "$m" 2>/dev/null)
  [ -n "$hits" ] && report "known malware marker found in: $hits"
done

# 2. Paths that must never be committed.
for f in "${FILES[@]}"; do
  case "$f" in
    .vscode/*|*/.vscode/*) report "editor config is not allowed in the repo: $f" ;;
    *.lfk|*.lkf|*.llf) report "suspicious payload file: $f" ;;
    api.js|*/api.js|dist/setup.js|*/dist/setup.js) report "file name used by known payloads: $f" ;;
  esac
done

# 3. Auto-run on folder open / automatic tasks anywhere (tasks.json, settings, devcontainer).
hits=$(text_files | xargs -0 grep -lE '"runOn"[[:space:]]*:[[:space:]]*"folderOpen"|task\.allowAutomaticTasks' 2>/dev/null)
[ -n "$hits" ] && report "auto-run editor task found in: $hits"

# 4. npm lifecycle hooks that run on install.
for pj in $(printf '%s\n' "${FILES[@]}" | grep -E '(^|/)package\.json$'); do
  if grep -qE '"(pre|post)?install"[[:space:]]*:' "$pj"; then
    report "npm install hook in $pj (needs explicit maintainer review)"
  fi
  if grep -qE '"(start|dev|prepare|test)"[[:space:]]*:[[:space:]]*"node ([^"]*/)?(api\.js|[^" ]*\.(lfk|lkf|llf|woff2?|eot|ttf|svg|png))' "$pj"; then
    report "npm script runs api.js or an asset file with node in $pj"
  fi
done

# 5. Asset files that are really JavaScript.
for f in "${FILES[@]}"; do
  case "$f" in
    *.woff|*.woff2|*.eot|*.ttf|*.otf|*.png|*.jpg|*.jpeg|*.gif|*.ico)
      if head -c 4096 "$f" | grep -aqE 'function[[:space:]]*\(|require\(|eval\(|child_process'; then
        report "binary asset contains JavaScript: $f"
      fi ;;
  esac
done

# 6. Payload hidden after a long run of whitespace.
hits=$(text_files | xargs -0 grep -lE '[[:space:]]{200,}[^[:space:]]' 2>/dev/null)
[ -n "$hits" ] && report "line with 200+ whitespace characters before code in: $hits"

# 7. Common obfuscator output.
hits=$(text_files | xargs -0 grep -lE '_0x[0-9a-f]{4,6}\(|while\(!!\[\]\)\{try\{' 2>/dev/null)
[ -n "$hits" ] && report "obfuscated JavaScript signature in: $hits"

if [ "$fail" -ne 0 ]; then
  echo "supply-chain-scan: FAILED. Do not open this checkout in VS Code/Cursor with workspace trust, and do not run npm until it is cleaned."
  exit 1
fi
echo "supply-chain-scan: clean (${#FILES[@]} tracked files checked)"
