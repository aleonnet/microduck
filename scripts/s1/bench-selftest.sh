#!/usr/bin/env bash
# The bench without a robot: every stage of `bench.sh` run against a stand-in `adb` that records
# what it was asked and answers just enough for the stage to finish. What this catches is the
# class of defect the robot session of 2026-10-30 paid for twice — a command the Android shell does
# not have (`ln`), a chroot entered with Android's PATH — before the robot is plugged in. What it
# cannot catch is what only the robot answers; that is the stage's own job.
#
# Usage: scripts/s1/bench-selftest.sh          (exit 0 = every stage ran and asked what it must)
set -euo pipefail
cd "$(dirname "$0")/../.."

WORK="$(mktemp -d "${TMPDIR:-/tmp}/s1-bench-selftest.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/bin" "$WORK/out"
export ADB_SHIM_LOG="$WORK/adb.log"
: > "$ADB_SHIM_LOG"

# The stand-in: logs every invocation; `pull` writes a stub so the stage's artefact lines resolve;
# a `shell` that dumps partitions answers with hash lines the backup stage can parse.
cat > "$WORK/bin/adb" <<'SHIM'
#!/bin/sh
echo "$*" >> "$ADB_SHIM_LOG"
shift 2   # -s <host>
case "$1" in
  get-state) echo device ;;
  forward|push) ;;
  pull) case "$2" in
          # A whole black 720p frame, so the camera stage's PNG conversion has real bytes to chew.
          *frame.nv12|*frame.raw) head -c 1382400 /dev/zero > "$3" ;;
          *) echo "shim" > "$3" ;;
        esac ;;
  shell)
    shift
    case "$*" in
      *"dd if="*) echo "0000000000000000000000000000000000000000000000000000000000000001  /sdcard/bk/system.img" ;;
      *getenforce*) echo Disabled; echo EXEC_IN_DATA_OK ;;
      *"test -"*) exit 0 ;;
      *gst-launch-1.0*num-buffers=300*) echo "Execution ended after 0:00:12.500000000" ;;
      *) echo "shim: $*" ;;
    esac ;;
esac
SHIM
chmod 755 "$WORK/bin/adb"
export PATH="$WORK/bin:$PATH"
# The e2e driver is pointed at ports nothing answers on: port 1 is reserved and refuses, so even a
# forward left by an earlier robot session on 9000 cannot make this self-test drive a robot.
export S1_HOST=shim:5555 S1_OUT="$WORK/out" S1_BAUD=921600 S1_UDP=127.0.0.1:1 S1_TCP=127.0.0.1:1

fail=0
run() { # run <stage> [args]: the stage must exit 0; `asked` then reads only what this stage sent
    local stage="$1"; shift
    ASKED_FROM=$(wc -c < "$ADB_SHIM_LOG" 2>/dev/null || echo 0)
    if scripts/s1/bench.sh "$stage" "$@" > "$WORK/$stage.txt" 2>&1; then
        echo "selftest: stage $stage ran"
    else
        echo "selftest: stage $stage FAILED (rc=$?):"; tail -20 "$WORK/$stage.txt"; fail=1
    fi
}
asked() { # asked <stage> <pattern>: the shim must have been asked it BY THAT STAGE (not an earlier one)
    if tail -c +"$((ASKED_FROM + 1))" "$ADB_SHIM_LOG" | grep -qF -- "$2"; then echo "selftest: $1 asked: $2"
    else echo "selftest: $1 NEVER asked: $2"; fail=1; fi
}

run capture
asked capture "strace -f -tt -e trace=ioctl,write,read,open,openat -s 256"
# A second run must not overwrite the first: the first is moved aside under its stamp.
run capture
n=$(ls -d "$S1_OUT"/capture-* 2>/dev/null | wc -l | tr -d ' ')
[ "$n" = "1" ] && [ -f "$S1_OUT"/capture-*/cmd.txt ] && echo "selftest: capture re-run archived the earlier run" || { echo "selftest: capture re-run did NOT archive the earlier run ($n archives)"; fail=1; }
run backup
asked backup "dd if="
run exec
asked exec "chroot /data/debian /bin/uname -a"
run stop
asked stop "stop dji_hdvt_uav"
# `drive` reaches the robot's socket through a forward the shim does not make, and S1_TCP points
# at a port that refuses: s1-e2e cannot connect, the stage records that and goes on — the
# behaviour wanted on a bench, and what lets the launch lines be checked here.
run drive
# After a reboot the chroot has no /dev: the stage must re-make the mounts before the daemon starts.
asked drive "|| mount -o bind /dev /data/debian/dev"
asked drive "chroot /data/debian /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin /opt/robot/bin/s1d --port /dev/ttyS3 --baud 921600 --sounds /opt/robot/sounds"
asked drive "chroot /data/debian /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin /opt/robot/bin/netpadd"
asked drive "robotctl health --json"
asked drive "robotctl quack"
asked drive "robotctl monitor --json"
run events
asked events "robotctl flashlight on --color red"
asked events "cat /dev/input/event0"
run camera
asked camera "v4l2-ctl -d /dev/video0 --list-formats-ext"
# Stage 7 must NOT stream with a generic tool (it rebooted the robot twice): no S_FMT, no mmap stream.
grep -qE -- "--set-fmt-video|--stream-mmap" "$ADB_SHIM_LOG" && { echo "selftest: camera still streams with v4l2-ctl"; fail=1; } || echo "selftest: camera stops before the stream"
[ -f "$S1_OUT/camera/blocked.txt" ] && echo "selftest: camera recorded the refusal" || { echo "selftest: camera did NOT record the refusal"; fail=1; }
run audio
asked audio "arecord -D hw:0,0 -f S16_LE -r 48000 -c 2 -d 60 /tmp/mic.wav"
asked audio "aplay -D hw:0,0 /mnt/system/audio/sound/engine_start.wav"
run frames
asked frames "|| mount -o bind /dev /data/debian/dev"
grep -q "gst-launch-1.0 v4l2src" "$ADB_SHIM_LOG" && { echo "selftest: frames still streams with gst-launch"; fail=1; } || echo "selftest: frames stops before the stream"
[ -f "$S1_OUT/frames/blocked.txt" ] && echo "selftest: frames recorded the refusal" || { echo "selftest: frames did NOT record the refusal"; fail=1; }
# The verify step over what the stages wrote: every artefact line must resolve.
grep -h "artefato:" "$WORK"/*.txt > "$WORK/record.md" || true
if scripts/s1/bench.sh verify-record "$WORK/record.md" > "$WORK/verify.txt" 2>&1; then
    echo "selftest: verify-record over $(grep -c OK "$WORK/verify.txt") artefacts of the stages"
else
    echo "selftest: verify-record FAILED:"; cat "$WORK/verify.txt"; fail=1
fi

[ "$fail" -eq 0 ] && echo "selftest: every stage ran and asked what it must" || { echo "selftest: FAILED" >&2; exit 1; }
