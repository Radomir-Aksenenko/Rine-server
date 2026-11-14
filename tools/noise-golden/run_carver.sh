#!/usr/bin/env bash
# Compiles tools/noise-golden/CarverGolden.java against the vanilla 26.3 server jar and writes
# crates/rine-worldgen/tests/carver_golden.bin, carver_sin_table.bin and carver_biomes.txt.
# Requires JDK 25 (javac, java). VANILLA overrides the vanilla-data directory (worktrees do not have it).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
VANILLA="${VANILLA:-$REPO/vanilla-data}"
BUILD="$HERE/build"
OUT_DIR="$REPO/crates/rine-worldgen/tests"

CP_UNIX="$VANILLA/versions/26.3/server-26.3.jar"
while IFS= read -r jar; do
    CP_UNIX="$CP_UNIX:$jar"
done < <(find "$VANILLA/libraries" -name '*.jar' | sort)
CP="$(cygpath -w -p "$CP_UNIX")"

mkdir -p "$BUILD" "$OUT_DIR"
javac -encoding UTF-8 -cp "$CP" -d "$(cygpath -w "$BUILD")" "$(cygpath -w "$HERE/CarverGolden.java")"
java -cp "$(cygpath -w "$BUILD");$CP" CarverGolden "$(cygpath -w "$OUT_DIR")"
