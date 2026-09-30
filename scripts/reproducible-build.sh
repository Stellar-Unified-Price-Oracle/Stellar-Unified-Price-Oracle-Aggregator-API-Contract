#!/usr/bin/env bash
# Byte-reproducible build check (#501).
#
# Builds the contract twice, from a clean target directory each time, in a
# neutralized environment, and fails unless the two WASM artifacts are
# byte-identical. Writes the artifact digests to build-artifacts/ so they can be
# published and compared against an independent rebuild.
#
# Canonical environment (see docs/reproducible-builds.md):
#   * toolchain   — rust-toolchain.toml (1.91.0), verified before building
#   * resolution  — --locked, so Cargo consumes the committed Cargo.lock
#   * timestamps  — SOURCE_DATE_EPOCH pinned to the commit time
#   * locale/TZ   — C / UTC, so ordering and formatting cannot vary
#   * incremental — off, so the build graph cannot carry state between runs
#
# Usage: ./scripts/reproducible-build.sh [--keep]
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
OUT_DIR="${OUT_DIR:-$ROOT/build-artifacts}"
WORK_DIR="${WORK_DIR:-$ROOT/target-repro}"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1

ARTIFACT="target/wasm32v1-none/release/price_oracle.wasm"

# --- canonical environment -------------------------------------------------
export LC_ALL=C
export LANG=C
export TZ=UTC
export CARGO_INCREMENTAL=0
export CARGO_TERM_COLOR=never
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "::error::cargo not found; install the pinned toolchain from rust-toolchain.toml" >&2
  exit 1
fi

PINNED_TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
ACTUAL_TOOLCHAIN="$(rustc --version | awk '{print $2}')"
if [ -n "$PINNED_TOOLCHAIN" ] && [ "$PINNED_TOOLCHAIN" != "$ACTUAL_TOOLCHAIN" ]; then
  echo "::error::toolchain $ACTUAL_TOOLCHAIN does not match rust-toolchain.toml ($PINNED_TOOLCHAIN)" >&2
  exit 1
fi

echo "==> canonical environment"
echo "    rustc          : $ACTUAL_TOOLCHAIN (pinned: $PINNED_TOOLCHAIN)"
echo "    SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
echo "    CARGO_INCREMENTAL: $CARGO_INCREMENTAL"
echo "    TZ/Locale      : $TZ / $LC_ALL"

build_once() {
  local name="$1"
  echo "==> clean build ($name)"
  rm -rf "${WORK_DIR:?}/$name"
  CARGO_TARGET_DIR="$WORK_DIR/$name" \
    cargo build -p price-oracle --target wasm32v1-none --release --locked
  local produced="$WORK_DIR/$name/wasm32v1-none/release/price_oracle.wasm"
  [ -f "$produced" ] || {
    echo "::error::build $name produced no $ARTIFACT" >&2
    exit 1
  }
  ( cd "$WORK_DIR/$name" && sha256sum "wasm32v1-none/release/price_oracle.wasm" )
}

HASH_A="$(build_once a | awk '{print $1}')"
HASH_B="$(build_once b | awk '{print $1}')"

mkdir -p "$OUT_DIR"
{
  echo "# Reproducible build digests (#501)"
  echo "# toolchain: $ACTUAL_TOOLCHAIN"
  echo "# SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
  echo "# artifact: $ARTIFACT"
  echo "$HASH_A  price_oracle.wasm"
} > "$OUT_DIR/wasm-hashes.txt"

if [ "$HASH_A" != "$HASH_B" ]; then
  echo "::error::builds are NOT reproducible: $HASH_A != $HASH_B" >&2
  printf -- '- build A: `%s`\n- build B: `%s`\n' "$HASH_A" "$HASH_B" \
    > "$OUT_DIR/reproducible-build.md"
  exit 1
fi

{
  echo "# Reproducible build report"
  echo
  echo "- toolchain: \`$ACTUAL_TOOLCHAIN\` (matches \`rust-toolchain.toml\`)"
  echo "- SOURCE_DATE_EPOCH: \`$SOURCE_DATE_EPOCH\`"
  echo "- artifact: \`$ARTIFACT\`"
  echo "- sha256: \`$HASH_A\`"
  echo "- two independent clean builds: **identical**"
} > "$OUT_DIR/reproducible-build.md"

echo "==> reproducible: $HASH_A"
echo "    digests written to $OUT_DIR/"

[ "$KEEP" -eq 1 ] || rm -rf "$WORK_DIR"
