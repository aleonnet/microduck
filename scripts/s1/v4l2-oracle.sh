#!/usr/bin/env bash
# The V4L2 layouts and ioctl numbers for the S1's CPU, from the ARM 32-bit compiler itself:
# v4l2-oracle.c built and run in a linux/arm/v7 bookworm container (the rootfs's own libc, glibc
# 2.36 with 32-bit time_t), its JSON written to s1d/tests/fixtures/v4l2_arm32.json, which a test
# in s1d holds the Rust structs to. Re-run after any change to v4l2-oracle.c; the diff is the review.
#
# Usage: scripts/s1/v4l2-oracle.sh            (needs Docker with linux/arm/v7 emulation)
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=s1d/tests/fixtures/v4l2_arm32.json
docker run --rm --platform linux/arm/v7 -v "$PWD/scripts/s1:/src:ro" debian:bookworm-slim sh -euc '
    apt-get update -qq >/dev/null && apt-get install -y -qq gcc libc6-dev >/dev/null 2>&1
    gcc -Wall -Werror -o /tmp/oracle /src/v4l2-oracle.c && /tmp/oracle' > "$OUT.tmp"
python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["arch"]=="arm32" and d["sizeof_timeval"]==8, d' "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
echo "v4l2-oracle: $(python3 -I -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$OUT") values in $OUT"
