#!/usr/bin/env bash
# Refute the telemetry decoder: plant one wrong offset in the stock push's layout (the battery
# read ten bytes early, inside the ESC block), run the decode tests over the real fixtures, and
# demand that they FAIL naming the battery; then restore the source and prove it by hash. A layout
# whose tests pass with a wrong offset proves nothing — run after any change to `dds::StockPush`.
#
# Usage: scripts/s1/telemetry-refute.sh [<output file>]      exit 0 = the tests refused the planted offset
set -euo pipefail
cd "$(dirname "$0")/../.."

SRC=s1-duml/src/dds.rs
OUT="${1:-/dev/null}"
GOOD='const AT_BATTERY: usize = 82;'
BAD='const AT_BATTERY: usize = 72;'

RUN="$(mktemp "${TMPDIR:-/tmp}/s1-telemetry-refute.XXXXXX")"
before=$(shasum -a 256 "$SRC" | cut -d' ' -f1)
grep -qF "$GOOD" "$SRC" || { echo "refute: $SRC no longer holds '$GOOD'" >&2; exit 1; }
restore() { sed -i '' "s/$BAD/$GOOD/" "$SRC"; rm -f "$RUN"; }
trap restore EXIT

{
    echo "refute: planting '$BAD' in place of '$GOOD' in $SRC"
    sed -i '' "s/$GOOD/$BAD/" "$SRC"
    # The run's status is read from the program alone, never from a redirect.
    cargo test -p s1-duml --test stock_push > "$RUN" 2>&1 && status=0 || status=$?
    if [ "$status" -eq 0 ]; then
        echo "refute: the tests PASSED with a wrong battery offset planted — the layout tests cannot fail"; exit 1
    fi
    grep -q "battery" "$RUN" || { echo "refute: the failure does not name the battery:"; tail -30 "$RUN"; exit 1; }
    grep -E "panicked|test result|left:|right:" "$RUN" | head -8 | sed 's/^/refute: /'
    sed -i '' "s/$BAD/$GOOD/" "$SRC"
    after=$(shasum -a 256 "$SRC" | cut -d' ' -f1)
    if [ "$before" = "$after" ]; then
        echo "refute: source restored, sha256 $after equal before and after"
    else
        echo "refute: source NOT restored ($before -> $after)"; exit 1
    fi
    cargo test -p s1-duml --test stock_push > "$RUN" 2>&1 || { echo "refute: tests do not pass after the restore:"; tail -20 "$RUN"; exit 1; }
    echo "refute: the planted offset was refused; the layout tests can fail"
} | tee "$OUT"
