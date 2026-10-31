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
          *frame_00*.nv12|*frame.nv12|*frame.raw) head -c 1382432 /dev/zero > "$3" ;;
          *) echo "shim" > "$3" ;;
        esac ;;
  shell)
    shift
    case "$*" in
      *"dd if="*) echo "0000000000000000000000000000000000000000000000000000000000000001  /sdcard/bk/system.img" ;;
      *getenforce*) echo Disabled; echo EXEC_IN_DATA_OK ;;
      *"test -"*) exit 0 ;;
      *"/opt/robot/bin/s1-cam "*) echo '{"streamed": true, "rate": {"fps": 29.9}, "frames_saved": [], "stalled": false}' ;;
      *"echo s1d_already_up"*) ;;
      *"echo s1d_up"*) echo s1d_up ;;
      *gst-launch-1.0*num-buffers=300*) echo "Execution ended after 0:00:12.500000000" ;;
      *) echo "shim: $*" ;;
    esac ;;
esac
SHIM
chmod 755 "$WORK/bin/adb"
export PATH="$WORK/bin:$PATH"
# The e2e driver is pointed at ports nothing answers on: port 1 is reserved and refuses, so even a
# forward left by an earlier robot session on 9000 cannot make this self-test drive a robot.
# The speaker control's name carries a space, as stage 8 found it: the launch line must quote it.
export S1_HOST=shim:5555 S1_OUT="$WORK/out" S1_BAUD=921600 S1_UDP=127.0.0.1:1 S1_TCP=127.0.0.1:1 S1_VOLUME_CONTROL="Speaker Volume"

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
asked drive "chroot /data/debian /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin /opt/robot/bin/s1d --port /dev/ttyS3 --baud 921600 --sounds /opt/robot/sounds --volume-control 'Speaker Volume'"
asked drive "pidof s1d >/dev/null && echo s1d_up"
asked drive "chroot /data/debian /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin /opt/robot/bin/netpadd"
asked drive "robotctl health --json"
asked drive "robotctl quack"
asked drive "robotctl monitor --json"
run events
asked events "robotctl flashlight on --color red"
asked events "cat /dev/input/event0"
run camera
asked camera "v4l2-ctl -d /dev/video0 --list-formats-ext"
# The frame comes from s1-cam replaying the stock's sequence (three frames, ten seconds of rate), never from a generic tool.
asked camera "/opt/robot/bin/s1-cam --device /dev/video0 --warmup 90 --frames 3 --seconds 10 --out /tmp/cam"
[ -f "$S1_OUT/camera/frame.png" ] && echo "selftest: camera converted the frame to PNG" || { echo "selftest: camera did NOT produce frame.png"; fail=1; }
# Stage 7 must NOT stream with a generic tool (it rebooted the robot twice): no S_FMT, no mmap stream.
grep -qE -- "--set-fmt-video|--stream-mmap" "$ADB_SHIM_LOG" && { echo "selftest: camera still streams with v4l2-ctl"; fail=1; } || echo "selftest: camera stops before the stream"
[ -f "$S1_OUT/camera/blocked.txt" ] && { echo "selftest: camera recorded a refusal although s1-cam is installed"; fail=1; } || echo "selftest: camera ran s1-cam instead of refusing"
run audio
asked audio "arecord -D hw:0,0 -f S16_LE -r 48000 -c 2 -d 60 /tmp/mic.wav"
asked audio "aplay -D hw:0,0 /mnt/system/audio/sound/engine_start.wav"
run telemetry
# The stock controller process relaunched under strace from its first write, then init's service restored.
asked telemetry "strace -f -tt -e trace=write -s 256 -o /data/hdvt_exec.strace /system/bin/dji_hdvt_uav -g"
asked telemetry "kill -9 \$(/system/xbin/busybox pidof dji_hdvt_uav)"
asked telemetry "start dji_hdvt_uav"
tail -c +"$((ASKED_FROM + 1))" "$ADB_SHIM_LOG" | grep -qE "s1d|s1ctl|robotctl|forward" && { echo "selftest: telemetry stage ran something of ours"; fail=1; } || echo "selftest: telemetry stage runs nothing of ours"
run telemetry-gimbal
asked telemetry-gimbal "strace -f -tt -e trace=read -s 256 -o /data/hdvt_gimbal.strace -p"
# telemetry-free: the stock stopped and the daemons up as stages 4 and 5 do, s1d's reads under
# strace, the driver pointed at a port that refuses — the stage records that and goes on.
run telemetry-free
asked telemetry-free "stop dji_hdvt_uav"
asked telemetry-free "chroot /data/debian /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin /opt/robot/bin/s1d --port /dev/ttyS3 --baud 921600 --sounds /opt/robot/sounds --volume-control 'Speaker Volume'"
asked telemetry-free "pidof s1d >/dev/null && echo s1d_up"
asked telemetry-free "strace -f -tt -e trace=read -s 256 -o /data/s1d_free.strace -p"
grep -q "telemetry-free rc=2" "$WORK/telemetry-free.txt" && echo "selftest: telemetry-free recorded the unreachable daemon" || { echo "selftest: telemetry-free did NOT record the unreachable daemon"; fail=1; }
# The two judges over cases built in the scripts themselves.
python3 -I scripts/s1/telemetry-free.py --selftest || fail=1
python3 -I scripts/s1/duml-strace.py --selftest || fail=1
# And the driver end to end against s1d --fake fed the robot's own pushes (fixtures cut from the
# 2026-10-30 captures): arm, sweep, sample, judge — the only part the shim cannot stand in for.
cargo build -p s1d --bin s1d >/dev/null 2>&1 || { echo "selftest: cargo build s1d failed"; fail=1; }
FAKE="$WORK/fake"; mkdir -p "$FAKE"; : > "$FAKE/in.bin"
SOCK="$(mktemp -u /tmp/s1-free-XXXXXX).sock"
RUST_LOG=info target/debug/s1d --fake "$FAKE/wire.bin" --fake-in "$FAKE/in.bin" --socket "$SOCK" > "$FAKE/s1d.log" 2>&1 &
S1D_PID=$!
for _ in $(seq 1 50); do [ -S "$SOCK" ] && break; sleep 0.1; done
cat s1-duml/tests/fixtures/gimbal_push_yaw_max_2026-10-30.bin s1-duml/tests/fixtures/dds_push_stock_idle_2026-10-30.bin >> "$FAKE/in.bin"
if scripts/s1/telemetry-free.py run --socket "$SOCK" --out "$FAKE/samples.ndjson" --sweep 2 --hold 1 --rate 10 > "$FAKE/verdict.txt" 2>&1; then
    grep -q "^body_still" "$FAKE/verdict.txt" && grep -q "^yaw_relation" "$FAKE/verdict.txt" && echo "selftest: telemetry-free drove s1d --fake and judged the samples" || { echo "selftest: telemetry-free ran but judged nothing:"; fail=1; }
    sed 's/^/selftest:   /' "$FAKE/verdict.txt"
    # The arm the driver asked for must have put the gimbal in work mode 0 on the wire (0x04/0x4c
    # 00 00) — the frame delivery 2 added, seen here through the driver's own robot.enable.
    python3 -I - "$FAKE/wire.bin" <<'PY' || fail=1
import sys
b = open(sys.argv[1], 'rb').read(); i = 0; modes = []
while True:
    i = b.find(b'\x55', i)
    if i < 0 or i + 13 > len(b): break
    if b[i + 9] == 0x04 and b[i + 10] == 0x4c: modes.append(b[i + 11:i + 13].hex())
    i += 1
ok = modes[:1] == ['0000']
print(('selftest: the arm put gimbal work mode 0 on the wire: %s' % modes) if ok else ('selftest: NO gimbal free mode on the wire at arm: %s' % modes))
sys.exit(0 if ok else 1)
PY
else
    echo "selftest: telemetry-free against s1d --fake FAILED:"; cat "$FAKE/verdict.txt"; tail -5 "$FAKE/s1d.log"; fail=1
fi
kill "$S1D_PID" 2>/dev/null; wait "$S1D_PID" 2>/dev/null || true; rm -f "$SOCK"
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
