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
# plant <file> <good text> <bad text> <command> <word the failure must name>: the command must FAIL
# with the bad text in place and pass again once the file is restored, which the hash proves.
plant() {
    local f="$1" good="$2" bad="$3" cmd="$4" word="$5" sha status
    sha=$(shasum -a 256 "$f" | cut -d' ' -f1)
    grep -qF "$good" "$f" || { echo "refute: $f no longer holds '$good'"; exit 1; }
    python3 -I -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); s=p.read_text(); assert s.count(sys.argv[2])==1; p.write_text(s.replace(sys.argv[2], sys.argv[3]))' "$f" "$good" "$bad"
    echo "refute: planting '$bad' in $f"
    $cmd > "$RUN" 2>&1 && status=0 || status=$?
    python3 -I -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_text(p.read_text().replace(sys.argv[3], sys.argv[2]))' "$f" "$good" "$bad"
    [ "$(shasum -a 256 "$f" | cut -d' ' -f1)" = "$sha" ] || { echo "refute: $f NOT restored"; exit 1; }
    if [ "$status" -eq 0 ]; then echo "refute: $cmd PASSED with the defect planted — it cannot fail"; exit 1; fi
    grep -q "$word" "$RUN" || { echo "refute: the failure does not name $word:"; tail -10 "$RUN"; exit 1; }
    $cmd > "$RUN" 2>&1 || { echo "refute: $cmd does not pass after the restore:"; tail -10 "$RUN"; exit 1; }
    echo "refute: $f restored (sha256 equal); the defect was refused naming $word"
}

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
    # The two judges of stage telemetry-free, each with its own defect: the head verdict with the
    # duck/DJI sign dropped (a head that turned the wrong way would pass), and the pairing report
    # with the attitude read twelve bytes early (inside the status word and the IMU).
    plant scripts/s1/telemetry-free.py "errs.append((cmd, last, abs(wrap(last - (-cmd)))))" "errs.append((cmd, last, abs(wrap(last - cmd))))" "python3 -I scripts/s1/telemetry-free.py --selftest" "head_obeyed"
    plant scripts/s1/duml-strace.py "AT_STOCK_ATTITUDE = 132" "AT_STOCK_ATTITUDE = 120" "python3 -I scripts/s1/duml-strace.py --selftest" "gimbal_vs_attitude"
    # The plausibility guard with its window opened to anything: the relaunched boot's 49 mV
    # would pass as a pack, and the layout test over that real push must say so.
    plant s1-duml/src/dds.rs "Self::PLAUSIBLE_MV.contains(&self.adc_value) && self.percent <= 100" "self.percent <= 100" "cargo test -p s1-duml --test stock_push" "implausible"
    plant scripts/s1/telemetry-free.py 'lines.append(f"body_still undecided: no wheel speeds in the samples' 'lines.append(f"body_still yes: no wheel speeds in the samples' "python3 -I scripts/s1/telemetry-free.py --selftest" "body_still undecided"
} | tee "$OUT"
