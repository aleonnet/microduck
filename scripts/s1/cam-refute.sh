#!/usr/bin/env bash
# Refute the camera's guards: plant one wrong number in the recording (the format's size the driver
# answered) and demand the sequence tests fail naming the real one; plant one wrong ioctl number and
# demand the oracle test fails naming it. Each file restored and proven by hash. Run after any
# change to s1d/src/{v4l2,cam}.rs.
#
# Usage: scripts/s1/cam-refute.sh      exit 0 = both defects refused
set -euo pipefail
cd "$(dirname "$0")/../.."
RUN="$(mktemp "${TMPDIR:-/tmp}/s1-cam-refute.XXXXXX")"; trap 'rm -f "$RUN"' EXIT
plant() { # plant <file> <good> <bad> <command> <word>
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
plant s1d/src/cam.rs "pub const SIZE_IMAGE: u32 = 1_382_432;" "pub const SIZE_IMAGE: u32 = 1_382_400;" "cargo test -p s1d --lib cam" "1382432"
plant s1d/src/v4l2.rs "pub const VIDIOC_S_FMT: u32 = 0xc0cc_5605;" "pub const VIDIOC_S_FMT: u32 = 0xc0cc_5606;" "cargo test -p s1d --lib v4l2" "VIDIOC_S_FMT"
