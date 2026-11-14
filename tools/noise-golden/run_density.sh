#!/usr/bin/env bash
# Compiles tools/noise-golden/DensityGolden.java against the vanilla 26.3 server jar and writes
# crates/rine-worldgen/tests/density_points.txt and density_chunks.bin. Requires JDK 25 (javac, java).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
VANILLA="$REPO/vanilla-data"
BUILD="$HERE/build"
OUT_DIR="$REPO/crates/rine-worldgen/tests"

CP_UNIX="$VANILLA/versions/26.3/server-26.3.jar"
while IFS= read -r jar; do
    CP_UNIX="$CP_UNIX:$jar"
done < <(find "$VANILLA/libraries" -name '*.jar' | sort)
CP="$(cygpath -w -p "$CP_UNIX")"

mkdir -p "$BUILD" "$OUT_DIR"
javac -encoding UTF-8 -cp "$CP" -d "$(cygpath -w "$BUILD")" "$(cygpath -w "$HERE/DensityGolden.java")"
java -cp "$(cygpath -w "$BUILD");$CP" DensityGolden "$(cygpath -w "$OUT_DIR")"
