#!/usr/bin/env bash
# Refute the end-to-end bench: plant one wrong byte in what an item expects on the wire, run that
# item, and demand that it FAILS; then restore the source and prove it by hash. A judge that cannot
# fail proves nothing, and this is the executable form of that sentence — run after any change to
# `s1-e2e`, and its output is the record's artefact.
#
# Usage: scripts/s1/e2e-refute.sh [<output file>]      exit 0 = the bench refused the planted byte
set -euo pipefail
cd "$(dirname "$0")/../.."

SRC=s1d/src/bin/s1-e2e.rs
OUT="${1:-/dev/stdout}"
GOOD='551604fcc9040000403fb2070805640000006400bfa0'   # F2 recenter, DJI's own bytes
BAD='551604fcc9040000403fb2070805640000006400bfa1'    # the last byte off by one

RUN="$(mktemp "${TMPDIR:-/tmp}/s1-e2e-refute.XXXXXX")"   # the planted run's own output
before=$(shasum -a 256 "$SRC" | cut -d' ' -f1)
grep -q "$GOOD" "$SRC" || { echo "refute: $SRC no longer expects $GOOD" >&2; exit 1; }
restore() { sed -i '' "s/$BAD/$GOOD/" "$SRC"; rm -f "$RUN"; }
trap restore EXIT

{
    echo "refute: planting $BAD in place of $GOOD in $SRC"
    sed -i '' "s/$GOOD/$BAD/" "$SRC"
    cargo build -p s1d --bin s1-e2e >/dev/null 2>&1
    # The run's status is read from the program alone: a redirect that could not be opened would
    # also be "non-zero", and that must not count as the item failing.
    target/debug/s1-e2e fake --only F2 > "$RUN" 2>&1 && status=0 || status=$?
    if [ "$status" -eq 0 ]; then
        echo "refute: the item PASSED with a wrong byte planted — the judge cannot fail"; exit 1
    fi
    grep -q "0 of 1 items passed" "$RUN" || { echo "refute: the run did not reach its table:"; cat "$RUN"; exit 1; }
    grep -E "FAIL|items passed" "$RUN" | sed 's/^/refute: /'
    sed -i '' "s/$BAD/$GOOD/" "$SRC"
    after=$(shasum -a 256 "$SRC" | cut -d' ' -f1)
    cargo build -p s1d --bin s1-e2e >/dev/null 2>&1
    if [ "$before" = "$after" ]; then
        echo "refute: source restored, sha256 $after equal before and after"
    else
        echo "refute: source NOT restored ($before -> $after)"; exit 1
    fi
    echo "refute: the planted byte was refused; the bench can fail"
} | tee "$OUT"
