#!/usr/bin/env bash
# Compiles tools/noise-golden/Golden.java against the vanilla 26.3 server jar and
# writes crates/rine-worldgen/tests/golden.txt. Requires JDK 25 on PATH (javac, java).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
VANILLA="$REPO/vanilla-data"
BUILD="$HERE/build"
OUT="$REPO/crates/rine-worldgen/tests/golden.txt"

# Classpath: vanilla jar + every library jar (Windows-style, ';' separated for the JVM).
CP_UNIX="$VANILLA/versions/26.3/server-26.3.jar"
while IFS= read -r jar; do
    CP_UNIX="$CP_UNIX:$jar"
done < <(find "$VANILLA/libraries" -name '*.jar' | sort)
CP="$(cygpath -w -p "$CP_UNIX")"

mkdir -p "$BUILD" "$(dirname "$OUT")"
javac -encoding UTF-8 -cp "$CP" -d "$(cygpath -w "$BUILD")" "$(cygpath -w "$HERE/Golden.java")"
java -cp "$(cygpath -w "$BUILD");$CP" Golden "$(cygpath -w "$OUT")"
