#!/usr/bin/env bash
# The S1 bench, in stages: what the plan `docs/project/s1-bringup.md` records, one subcommand each,
# every stage leaving its artefacts under $OUT/<stage>/ and printing what it produced. The order
# matters and is the plan's: capture while the stock firmware is alive, back up before anything is
# stopped, prove execution before anything is pushed, free the UART before driving.
#
# Nothing here touches /system or flashes anything; `restore` is a reboot. Robot wheels must be off
# the ground for `drive`. `verify-record` is the only stage that runs without a robot: it checks
# that every artefact a record cites exists with the sha256 it claims.
#
# Usage: scripts/s1/bench.sh <stage> [args]      stages: help | capture | backup | exec | stop | drive |
#        items | events | camera | audio | app | restore | frames | infer | telemetry | telemetry-gimbal |
#        telemetry-free | e2e |
#        verify-record <record.md>
# Env:   S1_HOST (default 192.168.1.236:5555) · S1_OUT (default ~/Development/RoboMaster-S1/out/<date>-poc2)
#        S1_CHROOT (default /data/debian) · S1_BAUD (from capture) · S1_ROOTFS (tar from build-rootfs.sh)
#        S1_UDP (the robot's netpadd, default <host without port>:4210) · S1_TCP (the forwarded socket,
#        default 127.0.0.1:9000) · S1_VOLUME_CONTROL (the speaker's mixer control, from stage 8's dump)
set -euo pipefail
cd "$(dirname "$0")/../.."   # `cargo s1` (the alias in .cargo/config.toml) and target/ are the repo's

HOST="${S1_HOST:-192.168.1.236:5555}"
UDP="${S1_UDP:-${HOST%%:*}:4210}"
TCP="${S1_TCP:-127.0.0.1:9000}"
OUT="${S1_OUT:-$HOME/Development/RoboMaster-S1/out/$(date +%Y-%m-%d)-poc2}"
CHROOT="${S1_CHROOT:-/data/debian}"
BB=/system/xbin/busybox
A="adb -s $HOST"
# The chroot's own PATH: the daemons spawn `aplay` and `amixer` by name, and a chroot entered from
# the Android shell inherits Android's PATH (`/sbin:/system/bin:/system/xbin`, measured
# 2026-10-30 in /proc/<pid>/environ — `robot.sound` answered "aplay: No such file or directory").
DEBIAN_PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
CHROOT_ENV="$BB chroot $CHROOT /usr/bin/env PATH=$DEBIAN_PATH"
stage="${1:-help}"; shift || true

stages() {
    cat <<'LIST'
 1 capture   with the stock firmware ALIVE: cpuinfo, partitions, mounts, df, getenforce, ps, stty of
             ttyS3; strace of dji_hdvt_uav and dji_camera while YOU use the app (camera, sound, hit
             the armour, change LED colours, press the button, person recognition) with top alongside
 2 backup    dd every partition → staging → pull → sha256 on the Mac (SHA256SUMS)
 3 exec      can our binaries run? busybox copied to /data; chroot into the rootfs; uname, ldd
 4 stop      stop dji_hdvt_uav + dji_monitor; list who still holds ttyS3
 5 drive     s1d + netpadd + robotctl health in the chroot; then `s1-e2e robot`: every item of the
             body, in the order the fake run proved — the pad over UDP to the robot's netpadd
             (wheels, turn, strafe, gimbal, lights, sounds, arm/disarm), the S1 list on the socket
             (follow, gimbal, goto, lights, blaster, armour, PWM, volume), telemetry, events
   items     `s1-e2e robot` alone, daemons already up; `items F2 F10` for some
 6 events    LEDs via robotctl flashlight in the chroot; the events window of s1-e2e; event0 in hex
 7 camera    stop dji_camera; v4l2-ctl formats; s1-cam replays the stock's sequence: 3 frames to files,
             10 s of rate; PNG on the Mac (no generic stream: it rebooted the robot twice)
 8 audio     amixer route; arecord 60 s stream; aplay while driving, top alongside
 9 app       observe what the app does with the stock stopped (notes)
10 restore   reboot; the app drives again; capture repeated shows hdvt_uav alive
11 frames    gst-launch v4l2src 720p and 360p for 30 s: fps and CPU (mediad's own element)
12 infer     s1-infer (the detector's ONNX path) on the S1 with libonnxruntime armv7, and on the Mac,
             over the same frames
   telemetry with the stock ALIVE, nothing of ours on the UART: dji_hdvt_uav relaunched under
             strace from its first instruction, 45 s — its boot-time DDS subscription (the subject
             list of the 176 B push). The app disconnects; no action needed from you.
   telemetry-gimbal  with the stock ALIVE and the app OPEN: 40 s of the UART reads while YOU drive
             the gimbal up and down to its pitch stops and sweep it left and right (the body follows
             the gimbal in the app's driving mode: yaw has no stop) — the 0x04/0x05 push against the
             manual's pitch limits, and its yaw against the quaternion
   telemetry-free  stock STOPPED, s1d armed (chassis work mode 1, gimbal work mode 0), WHEELS OFF
             THE GROUND: the head is swept left/right by robot.head for 30 s while s1.telemetry is
             sampled — the body must not follow (rpm 0, attitude yaw still) and ground yaw −
             chassis-relative yaw must equal the attitude yaw; then 20 s with the head at zero in
             which YOU tilt the robot nose-up ~15°, hold, nose-down, hold, level — which pitch field
             of the 0x04/0x05 push moves with the chassis. s1d's UART reads under strace alongside.
   e2e       no robot: `s1-e2e fake` — the same items against s1d --fake, netpadd, robotctl and
             s1ctl on this Mac, judged on the wire bytes (the pre-test of stages 5 and 6)
   verify-record <md>   check every `artefato:` line of a record: path exists, sha256 matches
LIST
}

say() { echo "[$stage] $*"; }
# A stage's directory. A re-run never overwrites an earlier run's artefacts (measured 2026-10-30:
# a second `capture` replaced eight files the record cited by hash, the morning's strace among
# them): when the directory already holds a run (`cmd.txt`, the command log every stage writes),
# it is moved aside under its own last-modified stamp first. `route.sh` placed by hand for stage 8
# goes with the archived run and is placed again.
dir() {
    if [ -f "$OUT/$1/cmd.txt" ]; then
        local stamp; stamp=$(date -r "$OUT/$1/cmd.txt" +%Y%m%d-%H%M%S)
        mv "$OUT/$1" "$OUT/$1-$stamp"; say "earlier $1 run archived as $OUT/$1-$stamp" >&2
    fi
    mkdir -p "$OUT/$1"; echo "$OUT/$1"
}
cmdlog() { echo "\$ $*" >> "$OUT/$stage/cmd.txt"; }
sh_robot() { cmdlog "adb shell $*"; $A shell "$@"; }
# The chroot's device, proc, sys and system trees. Stage 3 makes them; a reboot drops them, and a
# daemon started without `/dev/urandom` dies at once ("failed to generate random data", measured
# 2026-10-30 after stage 10). So every stage that enters the chroot re-makes them, guarded, before
# anything else — the same line, one place.
CHROOT_MOUNTS="$BB grep -q ' $CHROOT/dev ' /proc/mounts || mount -o bind /dev $CHROOT/dev; $BB grep -q ' $CHROOT/proc ' /proc/mounts || mount -t proc proc $CHROOT/proc; $BB grep -q ' $CHROOT/sys ' /proc/mounts || mount -t sysfs sys $CHROOT/sys; mkdir -p $CHROOT/mnt/system; $BB grep -q ' $CHROOT/mnt/system ' /proc/mounts || mount -o bind,ro /system $CHROOT/mnt/system"
need_chroot() { need_robot; sh_robot "$CHROOT_MOUNTS"; }
need_robot() { $A get-state >/dev/null 2>&1 || { echo "no adb at $HOST; enable root adb from the app's Lab first (pierluigizagaria/robomaster-s1-hacks)" >&2; exit 1; }; }
# s1-e2e against the robot: built here (natively, like s1-infer's Mac tier), the socket through the
# adb forward, the pad over UDP. Extra arguments narrow the items (`F2 F10 pad`).
e2e_robot() { local log="$1"; shift
    cargo build -p s1d --bin s1-e2e >/dev/null
    if [ "$#" -gt 0 ]; then target/debug/s1-e2e robot --tcp "$TCP" --udp "$UDP" --log "$log" --only "$@"
    else target/debug/s1-e2e robot --tcp "$TCP" --udp "$UDP" --log "$log"; fi
}
# s1d and netpadd up in the chroot (stages 5 and telemetry-free). `setsid` + `nohup` + stdin from
# /dev/null: measured on 2026-10-30, `nohup … &` alone left an empty log — the daemon died with the
# adb shell that started it. `setsid` alone still lost the race with the pty closing (netpadd, same
# day); the `sleep 1` inside the shell is what settles it. `--volume-control` only once stage 8's
# `amixer contents` has named the speaker's control: until then `s1.volume` is refused by name and
# the e2e item says so.
# The control's name is quoted for the robot's shell: it has a space ("Speaker Volume", stage 8),
# and unquoted it reached s1d as a second argument — the daemon refused it and died, and the stage
# went on against a dead socket (2026-10-30, 21:58). So the launch is followed by `pidof`: a daemon
# that did not come up ends the stage with the tail of its log, not with a riddle downstream.
start_daemons() {
    : "${S1_BAUD:?S1_BAUD is the baud the capture stage measured with stty}"
    # A re-run with the daemons still up keeps them: a second s1d would lose the socket and its
    # dying log would overwrite the live one's.
    if sh_robot "$BB pidof s1d >/dev/null && echo s1d_already_up" | grep -q s1d_already_up; then
        echo s1d_already_up | tee "$OUT/$stage/daemons.txt"; return 0
    fi
    sh_robot "($BB setsid $BB nohup $CHROOT_ENV /opt/robot/bin/s1d --port /dev/ttyS3 --baud $S1_BAUD --sounds /opt/robot/sounds ${S1_VOLUME_CONTROL:+--volume-control '$S1_VOLUME_CONTROL'} ${S1_ARGS:-} </dev/null >/data/s1d.log 2>&1 &); $BB sleep 1"
    sleep 2
    sh_robot "$BB pidof s1d >/dev/null && echo s1d_up || { echo S1D_DEAD; $BB tail -5 /data/s1d.log; }" | tee "$OUT/$stage/daemons.txt"
    grep -q s1d_up "$OUT/$stage/daemons.txt" || { say "s1d did not come up — its log's tail is above and in daemons.txt"; exit 1; }
    sh_robot "($BB setsid $BB nohup $CHROOT_ENV /opt/robot/bin/netpadd </dev/null >/data/netpadd.log 2>&1 &); $BB sleep 1"; sleep 1
}
# The stock body process and its monitor stopped, as stage 4 does; prints STILL_ALIVE or
# hdvt_uav_stopped (the controller keeps pushing the stock's telemetry either way, 2026-10-30).
STOP_STOCK="stop dji_hdvt_uav; stop dji_monitor; setprop ctl.stop dji_hdvt_uav; setprop ctl.stop dji_monitor; kill \$($BB pidof dji_hdvt_uav) 2>/dev/null; $BB sleep 1; $BB pidof dji_hdvt_uav >/dev/null && echo STILL_ALIVE || echo hdvt_uav_stopped"
art() { # art <file>: print the record line for an artefact
    local f="$1"; [ -f "$f" ] || { say "MISSING artefact $f"; return 1; }
    echo "artefato: $f sha256=$(shasum -a 256 "$f" | cut -d' ' -f1)" | tee -a "$OUT/artefatos.txt"
}

case "$stage" in
  help|-h|--help)
    echo "usage: $0 <stage> [args]"; stages ;;

  capture)
    need_robot; D=$(dir capture)
    for c in "cat /proc/cpuinfo" "cat /proc/partitions" "ls -l /dev/block/platform/*/by-name" "cat /proc/mounts" "df" "getenforce" "ps" "getprop" "$BB stty -F /dev/ttyS3 -a" "ls -l /dev/input /dev/video* /dev/snd"; do
        n=$(echo "$c" | tr ' /*' '___' | tr -s '_'); sh_robot "$c" > "$D/$n.txt" 2>&1 || true
    done
    # The mixer as the stock leaves it (the mic route lives there: dji_camera sets four controls by
    # id) — through the chroot's amixer when stage 3 has run, which it can with the stock alive.
    $A shell "test -x $CHROOT/usr/bin/amixer" 2>/dev/null && sh_robot "$BB chroot $CHROOT /usr/bin/amixer -c 0 contents" > "$D/mixer_stock_alive.txt" 2>&1 || true
    say "static capture done; now the strace. USE THE APP, one thing at a time, 3 s apart — the WHOLE list, nothing left for a later window (2026-10-30: a window spent on shots alone cost the follow modes):"
    say "  1 camera on · 2 a sound · 3 each armour plate · 4 LED colour, then each effect · 5 the controller button"
    say "  6 FIRE three times · 7 follow: chassis follows gimbal — turn the gimbal, the body turns (robot ON THE FLOOR)"
    say "  8 follow: gimbal follows chassis — drive, the gimbal stays · 9 FPV mode · 10 recentre gimbal · 11 drag the gimbal"
    say "  12 change the speaker volume · 13 speak into the mic · 14 person recognition"
    say "  at the very END: 15 power off by the button. 150 s."
    # Restarting `dji_scratch` here was tried once (2026-10-30 16:59): `stop dji_scratch` also
    # SIGKILLed `dji_sys` (the power daemon; init brought it back) and no `0x48` request reached
    # the UART in the window. Not done again until `dji_scratch.py` and the service group are read.
    sh_robot "top -n 1 -m 15" > "$D/top_before.txt" 2>&1 || true
    # Bounded: `strace -p` only ends with the traced process, so each gets a 70 s timeout and `wait`
    # returns; the top samples cover the same window.
    sh_robot "$BB timeout -t 150 strace -f -tt -e trace=ioctl,write,read,open,openat -s 256 -o /data/hdvt.strace -p \$($BB pidof dji_hdvt_uav) & $BB timeout -t 150 strace -f -tt -e trace=ioctl,write,read,open,openat -s 256 -o /data/sys.strace -p \$($BB pidof dji_sys) & $BB timeout -t 150 strace -f -tt -e trace=ioctl,write,read,open,openat -s 256 -o /data/camera.strace -p \$($BB pidof dji_camera) & for i in 1 2 3 4 5 6 7 8; do top -n 1 -m 10 >> /data/top_vision.txt; $BB sleep 10; done; wait" || true
    $A pull /data/hdvt.strace "$D/hdvt_uav.strace" >/dev/null; $A pull /data/camera.strace "$D/dji_camera.strace" >/dev/null; $A pull /data/sys.strace "$D/dji_sys.strace" >/dev/null 2>&1 || true; $A pull /data/top_vision.txt "$D/top_vision.txt" >/dev/null
    for f in "$D"/*.txt "$D"/*.strace; do art "$f"; done ;;

  backup)
    need_robot; D=$(dir backup)
    sh_robot "ls -l /dev/block/platform/*/by-name" | tee "$D/by-name.txt"
    sh_robot "df /data" | tee "$D/df.txt"
    # Staging on the microSD (vfat, 29.8 GB free, measured at stage 1): /data has 679 MB free and the
    # eMMC is 3.8 GB. The three big partitions (blackbox 1.9 GB, userdata 1.1 GB, cache 256 MB) are
    # dumped and hashed on the robot but stay on the card by default: at the measured 1.4 MB/s adb
    # pull they are 40 minutes; S1_PULL_SKIP="" pulls everything.
    STAGE="${S1_STAGE:-/sdcard/bk}"; SKIP="${S1_PULL_SKIP-blackbox userdata cache}"
    say "dumping each by-name partition to $STAGE on the robot, sha256 there, then pulling all but: ${SKIP:-nothing}"
    sh_robot "mkdir -p $STAGE; for p in /dev/block/platform/*/by-name/*; do n=\$($BB basename \$p); $BB dd if=\$p of=$STAGE/\$n.img bs=1M 2>/dev/null && $BB sha256sum $STAGE/\$n.img; done" | tr -d '\r' | tee "$D/SHA256SUMS.robot"
    mkdir -p "$D/bk"
    for n in $(awk '{ sub(".*/", "", $2); sub(/\.img$/, "", $2); print $2 }' "$D/SHA256SUMS.robot"); do
        case " $SKIP " in *" $n "*) say "left on the card: $n.img"; continue;; esac
        $A pull "$STAGE/$n.img" "$D/bk/$n.img" >/dev/null
    done
    ( cd "$D/bk" && shasum -a 256 *.img > ../SHA256SUMS )
    # Both sides reduced to `<sha> <name>`: busybox prints `/data/bk/<name>.img`, the Mac prints
    # `<name>.img` with two spaces, and comparing them raw never matched.
    # `tr -d '\r'`: adbd before the shell-v2 protocol (Android < 6) CRLFs its output.
    norm() { tr -d '\r' < "$1" | awk '{ sub(".*/", "", $2); print $1, $2 }' | sort; }
    # Compared over what was pulled; the lines left on the card are the robot's own hashes.
    diff <(norm "$D/SHA256SUMS.robot" | grep -F -f <(awk '{print $2}' "$D/SHA256SUMS")) <(norm "$D/SHA256SUMS") && say "SHA robot == Mac (pulled set)" || say "SHA MISMATCH — see $D"
    say "images stay on the robot's card at $STAGE (remove by hand when the Mac copy is complete)"
    for f in "$D"/SHA256SUMS "$D"/bk/*.img; do art "$f"; done ;;

  exec)
    need_robot; D=$(dir exec); ROOTFS="${S1_ROOTFS:-$HOME/Development/RoboMaster-S1/out/rootfs/rootfs-bookworm-armhf.tar}"
    { sh_robot "getenforce; cp /system/xbin/busybox /data/busybox && chmod 755 /data/busybox && /data/busybox true && echo EXEC_IN_DATA_OK || echo EXEC_IN_DATA_DENIED"; } | tee "$D/exec.txt"
    if grep -q DENIED "$D/exec.txt"; then say "execution in /data denied: the OWNER runs 'setenforce 0' (or we try the microSD); re-run this stage"; exit 2; fi
    if $A shell "test -f $CHROOT/bin/sh" 2>/dev/null; then
        say "rootfs already at $CHROOT (its /bin/sh exists); not pushing again"
    else
        [ -f "$ROOTFS" ] || { say "no rootfs at $ROOTFS; run scripts/s1/build-rootfs.sh"; exit 1; }
        say "pushing rootfs ($(du -h "$ROOTFS" | cut -f1)) and extracting into $CHROOT"
        $A push "$ROOTFS" /data/rootfs.tar >/dev/null
        sh_robot "mkdir -p $CHROOT && cd $CHROOT && $BB tar -xf /data/rootfs.tar && rm /data/rootfs.tar" | tee -a "$D/exec.txt"
    fi
    # the Android shell has no chroot or head: both are busybox applets here; mounts are guarded so the stage re-runs
    { sh_robot "$CHROOT_MOUNTS; $BB chroot $CHROOT /bin/true && echo CHROOT_OK; $BB chroot $CHROOT /bin/uname -a; $BB chroot $CHROOT /usr/bin/ldd --version | $BB head -1; $BB chroot $CHROOT /bin/cat /etc/debian_version"; } | tee -a "$D/exec.txt"
    art "$D/exec.txt" ;;

  stop)
    need_robot; D=$(dir stop)
    sh_robot "$STOP_STOCK" | tee "$D/stop.txt"
    sh_robot "for p in \$($BB ls /proc | $BB grep -E '^[0-9]+\$'); do $BB ls -l /proc/\$p/fd 2>/dev/null | $BB grep -q ttyS3 && echo \"ttyS3 held by \$p \$($BB cat /proc/\$p/comm)\"; done; echo owners_listed" | tee "$D/owners.txt"
    art "$D/stop.txt"; art "$D/owners.txt" ;;

  drive)
    D=$(dir drive); need_chroot
    say "starting s1d (baud ${S1_BAUD:-unset}) and netpadd in the chroot; WHEELS OFF THE GROUND"
    start_daemons
    # `robotctl health` exits non-zero on an unhealthy verdict; that is a result to record, not a
    # reason to stop the stage before the logs are pulled.
    sh_robot "$CHROOT_ENV /opt/robot/bin/robotctl health --json" > "$D/health.json" 2>&1 || say "robotctl health rc=$? — the verdict is in health.json"
    cat "$D/health.json"
    # The duck's own shell on the robot's socket, for the four items s1-e2e reaches by JSON from the
    # Mac (robotctl speaks only the unix socket): quack here, flashlight and monitor below and in
    # stage 6. LISTEN for the chirp.
    sh_robot "$CHROOT_ENV /opt/robot/bin/robotctl quack" | tee "$D/quack.txt" || say "robotctl quack rc=$?"
    # The Mac plays the pad and the shells: the pad as `duckctl udp-pad` sends it, straight to the
    # robot's netpadd over Wi-Fi; the socket through an adb forward (measured 2026-10-30:
    # `localfilesystem:` forwards to the chroot's unix socket).
    $A forward "tcp:${TCP##*:}" localfilesystem:$CHROOT/run/robotd.sock >/dev/null
    say "WHEELS OFF THE GROUND: every item now, in the fake run's order; answer each prompt"
    e2e_robot "$D/e2e.log" "$@" | tee "$D/e2e_console.txt" || say "s1-e2e rc=$? — the verdict per item is in e2e_console.txt"
    say "collecting robot.state for 10 s"
    sh_robot "$BB timeout -t 10 $CHROOT_ENV /opt/robot/bin/robotctl monitor --json" > "$D/monitor.log" 2>&1 || true
    $A pull /data/s1d.log "$D/s1d.log" >/dev/null; $A pull /data/netpadd.log "$D/netpadd.log" >/dev/null
    for f in "$D"/daemons.txt "$D"/health.json "$D"/quack.txt "$D"/e2e.log "$D"/e2e_console.txt "$D"/monitor.log "$D"/s1d.log "$D"/netpadd.log; do art "$f"; done ;;

  items)
    need_robot; D=$(dir items)
    $A forward "tcp:${TCP##*:}" localfilesystem:$CHROOT/run/robotd.sock >/dev/null
    say "the body's items through the daemons already up; WHEELS OFF THE GROUND; answer each prompt"
    e2e_robot "$D/e2e.log" "$@" | tee "$D/e2e_console.txt" || say "s1-e2e rc=$? — the verdict per item is in e2e_console.txt"
    $A pull /data/s1d.log "$D/s1d.log" >/dev/null
    for f in "$D"/e2e.log "$D"/e2e_console.txt "$D"/s1d.log; do art "$f"; done ;;

  e2e)
    D=$(dir e2e)
    say "no robot: s1d --fake, netpadd, robotctl and s1ctl on this Mac, every item judged on the wire"
    cargo build -p s1d -p netpadd -p robotctl >/dev/null
    target/debug/s1-e2e fake --dir "$D/work" --keep "$@" 2>&1 | tee "$D/e2e_fake.txt" || say "s1-e2e rc=$? — the verdict per item is in e2e_fake.txt"
    grep -q "items passed" "$D/e2e_fake.txt" || { say "s1-e2e did not run to its table: $(tail -3 "$D/e2e_fake.txt")"; exit 1; }
    art "$D/e2e_fake.txt"; art "$D/work/report.txt"; art "$D/work/wire.bin" ;;

  events)
    need_robot; D=$(dir events)
    for c in "on --color red" "on --color green" "on --color blue" "off"; do sh_robot "$CHROOT_ENV /opt/robot/bin/robotctl flashlight $c"; sleep 1; done | tee "$D/led_effects.txt"
    $A forward "tcp:${TCP##*:}" localfilesystem:$CHROOT/run/robotd.sock >/dev/null
    e2e_robot "$D/e2e.log" F10 F9 events | tee -a "$D/led_effects.txt" || true
    say "now the raw button device for 15 s: PRESS the button"
    sh_robot "$BB timeout -t 15 $BB cat /dev/input/event0 | $BB xxd" > "$D/event0.hex" 2>&1 || true
    $A pull /data/s1d.log "$D/s1d.log" >/dev/null
    grep -E "armour hit|infrared hit|button" "$D/s1d.log" | tee "$D/events_in_log.txt" || true
    for f in "$D"/led_effects.txt "$D"/e2e.log "$D"/event0.hex "$D"/s1d.log; do art "$f"; done ;;

  camera)
    D=$(dir camera); need_chroot
    sh_robot "stop dji_camera; setprop ctl.stop dji_camera; $BB sleep 1; $BB chroot $CHROOT /usr/bin/v4l2-ctl -d /dev/video0 --all" | tee "$D/v4l2_all.txt" || true
    sh_robot "$BB chroot $CHROOT /usr/bin/v4l2-ctl -d /dev/video0 --list-formats-ext" | tee "$D/formats.txt" || true
    # The driver refuses VIDIOC_S_FMT (measured 2026-10-30: "Invalid argument" with the device
    # already at 1280x720 NV12, Size Image 1382432): the frame is taken in the format the device
    # reports, and converted with that geometry. Paths given to a chroot program are the chroot's:
    # its /tmp is $CHROOT/tmp on the robot.
    sh_robot "$BB chroot $CHROOT /usr/bin/v4l2-ctl -d /dev/video0 --get-fmt-video" | tee "$D/fmt.txt" || true
    geo=$(awk '/Width\/Height/ {gsub(/[^0-9\/]/, "", $NF); print $NF}' "$D/fmt.txt" | tr '/' 'x'); geo=${geo:-1280x720}
    pix=$(awk -F"'" '/Pixel Format/ {print tolower($2)}' "$D/fmt.txt"); pix=${pix:-nv12}
    # NO STREAM HERE. A generic stream request (`v4l2-ctl --stream-mmap`, 17:01 and 17:5x on 2026-10-30)
    # rebooted the robot both times. The frame comes from `s1-cam`, which replays the stock camera
    # process's own call sequence (`camera/dji_camera_v4l2_decoded.strace`); until that binary exists
    # the stage records the refusal and stops here.
    # `s1-cam` (s1-push.sh installs it) replays the stock camera process's own sequence — input,
    # the three private controls, the format with its field and colourspace, five MMAP buffers —
    # holding every answer to the recording and switching the stream on only when all matched; a
    # divergence is JSON on stdout, exit 3, nothing streamed. 90 frames of warm-up (the first
    # ones off this sensor are dark, 2026-10-31), three frames saved, then 10 s of rate. Paths named to it are the chroot's. Without the binary the stage records the refusal.
    if $A shell "test -x $CHROOT/opt/robot/bin/s1-cam" 2>/dev/null; then
        sh_robot "$CHROOT_ENV /opt/robot/bin/s1-cam --device /dev/video0 --warmup 90 --frames 3 --seconds 10 --out /tmp/cam" | tee "$D/s1-cam.json" || say "s1-cam rc=$? — the report is in s1-cam.json"
        for i in 0 1 2; do $A pull "$CHROOT/tmp/cam/frame_00$i.nv12" "$D/frame_00$i.nv12" >/dev/null 2>&1 || true; done
        # NV12 720p as the driver declared it (1920 bytes a line, 1382432 a frame); the owner judges the PNG
        [ -f "$D/frame_000.nv12" ] && ffmpeg -loglevel error -y -f rawvideo -pix_fmt nv12 -s "$geo" -i "$D/frame_000.nv12" "$D/frame.png" || true
    else
        echo "stage 7 stops before the stream: a generic V4L2 stream rebooted the robot twice (2026-10-30); s1-cam is not installed (s1-push.sh)" | tee "$D/blocked.txt"
    fi
    for f in "$D"/formats.txt "$D"/fmt.txt "$D"/blocked.txt "$D"/s1-cam.json "$D"/frame_000.nv12 "$D"/frame_001.nv12 "$D"/frame_002.nv12 "$D"/frame.png; do [ -f "$f" ] && art "$f" || true; done ;;

  audio)
    D=$(dir audio); need_chroot
    sh_robot "$BB chroot $CHROOT /usr/bin/amixer -c 0 contents" > "$D/amixer_contents.txt" 2>&1 || true
    say "the stock's own sounds (F8): engine_start then the note 2C through aplay — LISTEN"
    sh_robot "$BB chroot $CHROOT /usr/bin/aplay -D hw:0,0 /mnt/system/audio/sound/engine_start.wav; $BB chroot $CHROOT /usr/bin/aplay -D hw:0,0 /mnt/system/audio/sound/media_sound_solmization_2C.wav" 2>&1 | tee "$D/aplay_stock_sounds.txt" || true
    say "route per the hdvt_uav strace (tinymix/amixer values) goes in $D/route.sh before this runs; PLAY MUSIC NOW"
    [ -f "$D/route.sh" ] && sh_robot "$(cat "$D/route.sh")"
    # The microphone is capture device 0 of card 0 — the stock camera process opens `pcmC0D0c`
    # (its strace from start, 2026-10-30); `hw:0,1` answered "read error: Invalid argument". The
    # device accepts S16_LE, 1–2 channels, 8–48 kHz (`arecord --dump-hw-params`), and the
    # microphone is on the RIGHT channel: a stereo probe measured channel 0 as a constant (RMS =
    # peak = −50.1 dB) and channel 1 as signal (mixer: `MIC2 Boost` on, `Dual-Mic` off). A mono
    # recording takes channel 0 and hears nothing — so: stereo at 48 kHz, the right channel split
    # off on the Mac. Paths given to the chroot's arecord are the chroot's ($CHROOT/tmp on the
    # robot); the tee runs outside.
    sh_robot "$BB chroot $CHROOT /usr/bin/arecord -D hw:0,0 -f S16_LE -r 48000 -c 2 -d 60 /tmp/mic.wav 2>&1 | $BB tee /data/stream_60s.txt" || true
    $A pull "$CHROOT/tmp/mic.wav" "$D/mic.wav" >/dev/null || true; $A pull /data/stream_60s.txt "$D/stream_60s.txt" >/dev/null || true
    [ -f "$D/mic.wav" ] && ffmpeg -loglevel error -y -i "$D/mic.wav" -af "pan=mono|c0=c1" "$D/mic_right.wav" && ffmpeg -hide_banner -loglevel info -i "$D/mic_right.wav" -af "astats=metadata=1:reset=0" -f null - 2>&1 | grep -E "RMS level|Peak level" | tee -a "$D/stream_60s.txt"
    say "aplay while driving: run stage drive in another terminal, then this plays 20 s and samples top"
    sh_robot "$BB chroot $CHROOT /usr/bin/aplay -D hw:0,0 /tmp/mic.wav & for i in 1 2 3 4; do top -n 1 -m 8; $BB sleep 5; done" > "$D/cpu_play_drive.txt" 2>&1 || true
    for f in "$D"/aplay_stock_sounds.txt "$D"/mic.wav "$D"/mic_right.wav "$D"/stream_60s.txt "$D"/cpu_play_drive.txt; do [ -f "$f" ] && art "$f"; done ;;

  app)
    D=$(dir app); say "with hdvt_uav/monitor stopped: open the app, note connect/video/control. Type notes, Ctrl-D:"; cat > "$D/obs.txt"; art "$D/obs.txt" ;;

  restore)
    need_robot; D=$(dir restore)
    # The forward of stages 5 and 6 is undone here, so a later run without a robot (the
    # self-test, `s1-e2e robot` by mistake) finds nothing listening on the Mac's port.
    $A forward --remove "tcp:${TCP##*:}" >/dev/null 2>&1 || true
    sh_robot "reboot" || true; say "rebooting; re-enable root adb from the app's Lab, then press Enter"; read -r _
    need_robot; sh_robot "$BB pidof dji_hdvt_uav && echo hdvt_uav_alive; getprop | $BB grep init.svc.dji" | tee "$D/after.txt"; art "$D/after.txt" ;;

  frames)
    D=$(dir frames); need_chroot
    # NO STREAM HERE either: `gst-launch v4l2src` is the same generic path that rebooted the robot
    # in stage 7. The rate is measured by `s1-cam` over the stock's own sequence; until it exists
    # the stage records the refusal. (`fpsdisplaysink` is absent from the rootfs anyway: the rate is
    # 300 buffers over the time the grabber reports.)
    echo "stage 11 stops before the stream: a generic V4L2 stream rebooted the robot twice (2026-10-30); fps waits for s1-cam" | tee "$D/blocked.txt"
    for f in "$D"/blocked.txt "$D"/fps_720p.txt "$D"/fps_360p.txt "$D"/sample.png; do [ -f "$f" ] && art "$f" || true; done ;;

  infer)
    D=$(dir infer); ORT="${S1_ORT:-$HOME/Development/RoboMaster-S1/out/onnxruntime}"; ORT_VER="${S1_ORT_VERSION:-1.28.0}"; MODEL="$OUT/duck_detect.onnx"
    [ -f "$MODEL" ] || curl --fail --location --silent --show-error -o "$MODEL" "https://huggingface.co/pollen-robotics/microduck-duck-detector/resolve/duck-v1/duck_detect.onnx"
    FRAMES="${S1_FRAMES:-$OUT/frames}"   # stage 11's PNG lands there; any directory of jpg/png works
    # `find`, not a pair of globs: an unmatched glob reaches `ls` literally and fails the guard on a
    # directory that holds only PNGs — which is what stage 11 leaves.
    [ -n "$(find "$FRAMES" -maxdepth 1 \( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' \) -print -quit 2>/dev/null)" ] || { say "no frames in $FRAMES — run stage 11 or set S1_FRAMES"; exit 1; }
    # `duck-bench` is the NPU path; the S1 has none. `s1-infer` is the same measurement over the
    # detector's ONNX path, cross-compiled here and built natively below for the other tier.
    # The on-board tier needs the robot; the Mac tier below runs regardless (one tier suffices).
    if [ -f "$ORT/libonnxruntime.so.$ORT_VER" ] && $A get-state >/dev/null 2>&1; then
        need_chroot
        cargo s1 -p s1d --bin s1-infer >/dev/null
        $A push target/armv7-unknown-linux-gnueabihf/release/s1-infer "$CHROOT/opt/robot/bin/s1-infer" >/dev/null
        $A push "$ORT/libonnxruntime.so.$ORT_VER" "$CHROOT/usr/lib/" >/dev/null; $A push "$MODEL" "$CHROOT/opt/robot/duck_detect.onnx" >/dev/null
        sh_robot "mkdir -p $CHROOT/opt/robot/frames"
        for f in "$FRAMES"/*.png "$FRAMES"/*.jpg; do [ -f "$f" ] && $A push "$f" "$CHROOT/opt/robot/frames/" >/dev/null; done
        sh_robot "ORT_DYLIB_PATH=/usr/lib/libonnxruntime.so.$ORT_VER $BB chroot $CHROOT /opt/robot/bin/s1-infer --model /opt/robot/duck_detect.onnx --frames /opt/robot/frames --hz 0" > "$D/onboard.txt" 2>&1 || true
    elif [ ! -f "$ORT/libonnxruntime.so.$ORT_VER" ]; then
        cp "$ORT/build.log" "$D/ort_build_failed.txt" 2>/dev/null || echo "no libonnxruntime armv7 and no build log" > "$D/ort_build_failed.txt"
    else
        say "no adb at $HOST: skipping the on-board tier, running the Mac tier"
    fi
    MAC_ORT="${S1_ORT_MAC:-$HOME/Development/RoboMaster-S1/out/onnxruntime-mac/onnxruntime-osx-arm64-$ORT_VER/lib/libonnxruntime.$ORT_VER.dylib}"
    cargo build --release -p s1d --bin s1-infer >/dev/null
    ORT_DYLIB_PATH="$MAC_ORT" target/release/s1-infer --model "$MODEL" --frames "$FRAMES" --hz 0 > "$D/offboard.txt" 2>&1 || true
    for f in "$D"/onboard.txt "$D"/offboard.txt "$D"/ort_build_failed.txt; do [ -f "$f" ] && art "$f"; done ;;

  telemetry)
    need_robot; D=$(dir telemetry)
    # (a) The stock's subscription is sent in the first milliseconds of `dji_hdvt_uav`: a strace
    # attached to the running service misses it (2026-10-30, `hdvt_boot.strace` holds only the
    # 1 Hz one). So the service is stopped and the binary relaunched under strace from its first
    # write, as init launches it (`init.rc`: `service dji_hdvt_uav /system/bin/dji_hdvt_uav -g`),
    # for 45 s; then init's service is started again. Writes only: nothing of ours touches the UART.
    # Ended by killing the relaunched process, not by `timeout` on strace: measured 2026-10-30
    # (20:50), busybox `timeout -t 45` sent strace its TERM and strace kept tracing for minutes;
    # the tracee's death is what ends strace. Our relaunch is the only dji_hdvt_uav alive here
    # (init's service was stopped), so `pidof` names it.
    say "the app will disconnect for about a minute: the stock controller process is relaunched under strace"
    sh_robot "stop dji_hdvt_uav; setprop ctl.stop dji_hdvt_uav; $BB sleep 1; (strace -f -tt -e trace=write -s 256 -o /data/hdvt_exec.strace /system/bin/dji_hdvt_uav -g >/dev/null 2>&1 &); $BB sleep 45; kill -9 \$($BB pidof dji_hdvt_uav); $BB sleep 2; start dji_hdvt_uav; $BB sleep 2; $BB pidof dji_hdvt_uav >/dev/null && echo hdvt_uav_restored || echo HDVT_NOT_RUNNING" | tee "$D/relaunch.txt"
    $A pull /data/hdvt_exec.strace "$D/hdvt_exec_writes.strace" >/dev/null
    scripts/s1/duml-strace.py "$D/hdvt_exec_writes.strace" dds_add_sub dds_add_node | tee "$D/dds_requests.txt"
    for f in "$D"/relaunch.txt "$D"/hdvt_exec_writes.strace "$D"/dds_requests.txt; do art "$f"; done ;;

  telemetry-gimbal)
    need_robot; D=$(dir telemetry-gimbal)
    # The gimbal's own attitude push, read off the UART while the owner drives the gimbal: pitch to
    # both stops (the manual's −20°/+35°; measured −21.2°/+36.8° on 2026-10-30), yaw swept left and
    # right — the chassis follows the gimbal in the app's driving mode, so yaw has no stop and is
    # held to the quaternion instead. The owner is told in the chat before this stage starts; the
    # window is 40 s from the first line below.
    say "WINDOW OPEN: 40 s — gimbal full UP, full DOWN, then sweep LEFT and RIGHT (the body will follow), then recentre"
    sh_robot "$BB timeout -t 40 strace -f -tt -e trace=read -s 256 -o /data/hdvt_gimbal.strace -p \$($BB pidof dji_hdvt_uav)" || true
    $A pull /data/hdvt_gimbal.strace "$D/hdvt_gimbal_reads.strace" >/dev/null
    scripts/s1/duml-strace.py "$D/hdvt_gimbal_reads.strace" gimbal_push | tee "$D/gimbal_pushes.txt"
    for f in "$D"/hdvt_gimbal_reads.strace "$D"/gimbal_pushes.txt; do art "$f"; done ;;

  telemetry-free)
    D=$(dir telemetry-free); need_chroot
    # Item 1 of 2026-10-30 (night), third delivery: the gimbal freed from the chassis measured on
    # the robot through s1d's own list. The stock is stopped (stage 4's line), the daemons come up
    # (stage 5's), and `telemetry-free.py` arms, sweeps the head by robot.head, samples
    # s1.telemetry and judges body_still / head_obeyed / yaw_relation / pitch_frame. s1d's reads of
    # the UART run under strace alongside (the attach `telemetry-gimbal` proved, 60 s) so the raw
    # 0x04/0x05 push — the pitch field at 2 is not in the list — is held against the attitude too.
    sh_robot "$STOP_STOCK" | tee "$D/stop.txt"
    start_daemons
    $A forward "tcp:${TCP##*:}" localfilesystem:$CHROOT/run/robotd.sock >/dev/null
    say "WHEELS OFF THE GROUND. 30 s: the head sweeps left and right on its own — the body must not try to follow."
    say "then 20 s with the head at zero: TILT the robot nose-up ~15 deg, hold 5 s, nose-down, hold 5 s, level."
    sh_robot "($BB timeout -t 60 strace -f -tt -e trace=read -s 256 -o /data/s1d_free.strace -p \$($BB pidof s1d) >/dev/null 2>&1 &); $BB sleep 1"
    scripts/s1/telemetry-free.py run --tcp "$TCP" --out "$D/samples.ndjson" --sweep 30 --hold 20 | tee "$D/verdict.txt" || say "telemetry-free rc=$? — the verdict is in verdict.txt"
    sleep 8   # the strace window outlasts the run by a few seconds
    $A pull /data/s1d_free.strace "$D/s1d_free_reads.strace" >/dev/null 2>&1 || true
    $A pull /data/s1d.log "$D/s1d.log" >/dev/null 2>&1 || true
    [ -f "$D/s1d_free_reads.strace" ] && scripts/s1/duml-strace.py "$D/s1d_free_reads.strace" gimbal_vs_attitude | tee "$D/gimbal_vs_attitude.txt"
    for f in "$D"/stop.txt "$D"/daemons.txt "$D"/samples.ndjson "$D"/verdict.txt "$D"/s1d_free_reads.strace "$D"/gimbal_vs_attitude.txt "$D"/s1d.log; do [ -f "$f" ] && art "$f" || true; done ;;

  verify-record)
    md="${1:?usage: bench.sh verify-record <record.md>}"
    [ -f "$md" ] || { echo "no such record: $md" >&2; exit 1; }
    n=0; bad=0
    while IFS= read -r line; do
        path=$(echo "$line" | sed -E 's/^.*artefato:[[:space:]]*([^[:space:]]+)[[:space:]]+sha256=([0-9a-f]{64}).*$/\1/')
        want=$(echo "$line" | sed -E 's/^.*artefato:[[:space:]]*([^[:space:]]+)[[:space:]]+sha256=([0-9a-f]{64}).*$/\2/')
        [ "$path" != "$line" ] || continue
        n=$((n+1)); path="${path/#\~/$HOME}"
        if [ ! -f "$path" ]; then echo "MISSING  $path"; bad=$((bad+1)); continue; fi
        have=$(shasum -a 256 "$path" | cut -d' ' -f1)
        if [ "$have" = "$want" ]; then echo "OK       $path"; else echo "SHA DIFF $path (record $want, file $have)"; bad=$((bad+1)); fi
    done < <(grep -E 'artefato:[[:space:]]*[^[:space:]]+[[:space:]]+sha256=[0-9a-f]{64}' "$md" || true)
    if [ "$n" -eq 0 ]; then echo "no artefacts recorded in $md: a record without evidence does not verify" >&2; exit 1; fi
    [ "$bad" -eq 0 ] && echo "verify-record: $n artefacts OK" || { echo "verify-record: $bad of $n failed" >&2; exit 1; } ;;

  *) echo "unknown stage: $stage" >&2; stages >&2; exit 1 ;;
esac
