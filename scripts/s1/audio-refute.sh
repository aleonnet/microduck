#!/usr/bin/env bash
# Refute stage 8's two guards, added after 2026-10-31 02:11 (the device busy, arecord failed, the
# stage pulled the day-before's recording as today's): plant the stage without the stop of the stock
# camera process and demand the self-test fail naming it; plant the stage without the abort on
# arecord's failure and demand the self-test fail on the busy run. bench.sh restored and proven by
# hash each time. Run after any change to the audio stage of scripts/s1/bench.sh.
#
# Usage: scripts/s1/audio-refute.sh      exit 0 = both defects refused
set -euo pipefail
cd "$(dirname "$0")/../.."
RUN="$(mktemp "${TMPDIR:-/tmp}/s1-audio-refute.XXXXXX")"; trap 'rm -f "$RUN"' EXIT
plant() { # plant <file> <good> <bad> <command> <word>
    local f="$1" good="$2" bad="$3" cmd="$4" word="$5" sha status
    sha=$(shasum -a 256 "$f" | cut -d' ' -f1)
    grep -qF -- "$good" "$f" || { echo "refute: $f no longer holds '$good'"; exit 1; }
    python3 -I -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); s=p.read_text(); assert s.count(sys.argv[2])==1; p.write_text(s.replace(sys.argv[2], sys.argv[3]))' "$f" "$good" "$bad"
    echo "refute: planting '$bad' in $f"
    $cmd > "$RUN" 2>&1 && status=0 || status=$?
    python3 -I -c 'import pathlib,sys; p=pathlib.Path(sys.argv[1]); p.write_text(p.read_text().replace(sys.argv[3], sys.argv[2]))' "$f" "$good" "$bad"
    [ "$(shasum -a 256 "$f" | cut -d' ' -f1)" = "$sha" ] || { echo "refute: $f NOT restored"; exit 1; }
    if [ "$status" -eq 0 ]; then echo "refute: $cmd PASSED with the defect planted — it cannot fail"; exit 1; fi
    grep -qF -- "$word" "$RUN" || { echo "refute: the failure does not name '$word':"; tail -10 "$RUN"; exit 1; }
    $cmd > "$RUN" 2>&1 || { echo "refute: $cmd does not pass after the restore:"; tail -10 "$RUN"; exit 1; }
    echo "refute: $f restored (sha256 equal); the defect was refused naming '$word'"
}
plant scripts/s1/bench.sh 'stop dji_camera; setprop ctl.stop dji_camera; $BB sleep 1; $BB rm -f' '$BB sleep 1; $BB rm -f' scripts/s1/bench-selftest.sh 'NEVER asked: stop dji_camera'
plant scripts/s1/bench.sh 'art "$D/stream_60s.txt"; exit 2 ;;' 'art "$D/stream_60s.txt" ;;' scripts/s1/bench-selftest.sh 'PASSED with the device busy'
# The channel: plant the left file cut from the RIGHT channel and demand the self-test fail naming mic_left.
plant scripts/s1/bench.sh 'for ch in left:c0 right:c1; do' 'for ch in left:c1 right:c1; do' scripts/s1/bench-selftest.sh 'mic_left.wav is not the left channel'
