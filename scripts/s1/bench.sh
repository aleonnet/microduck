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
#        events | camera | audio | app | restore | frames | infer | verify-record <record.md>
# Env:   S1_HOST (default 192.168.1.236:5555) · S1_OUT (default ~/Development/RoboMaster-S1/out/<date>-poc2)
#        S1_CHROOT (default /data/debian) · S1_BAUD (from capture) · S1_ROOTFS (tar from build-rootfs.sh)
set -euo pipefail
cd "$(dirname "$0")/../.."   # `cargo s1` (the alias in .cargo/config.toml) and target/ are the repo's

HOST="${S1_HOST:-192.168.1.236:5555}"
OUT="${S1_OUT:-$HOME/Development/RoboMaster-S1/out/$(date +%Y-%m-%d)-poc2}"
CHROOT="${S1_CHROOT:-/data/debian}"
BB=/system/xbin/busybox
A="adb -s $HOST"
stage="${1:-help}"; shift || true

stages() {
    cat <<'LIST'
 1 capture   with the stock firmware ALIVE: cpuinfo, partitions, mounts, df, getenforce, ps, stty of
             ttyS3; strace of dji_hdvt_uav and dji_camera while YOU use the app (camera, sound, hit
             the armour, change LED colours, press the button, person recognition) with top alongside
 2 backup    dd every partition → staging → pull → sha256 on the Mac (SHA256SUMS)
 3 exec      can our binaries run? busybox copied to /data; chroot into the rootfs; uname, ldd
 4 stop      stop dji_hdvt_uav + dji_monitor; list who still holds ttyS3
 5 drive     s1d + netpadd + robotctl health in the chroot; you drive with `duckctl udp-pad <ip>`
 6 events    LEDs via robotctl flashlight and the full 0x3f/0x33; armour hits, IR, button in the log
 7 camera    stop dji_camera; v4l2-ctl formats; one frame to a file; PNG on the Mac
 8 audio     amixer route; arecord 60 s stream; aplay while driving, top alongside
 9 app       observe what the app does with the stock stopped (notes)
10 restore   reboot; the app drives again; capture repeated shows hdvt_uav alive
11 frames    gst-launch v4l2src 720p and 360p for 30 s: fps and CPU (mediad's own element)
12 infer     s1-infer (the detector's ONNX path) on the S1 with libonnxruntime armv7, and on the Mac,
             over the same frames
   verify-record <md>   check every `artefato:` line of a record: path exists, sha256 matches
LIST
}

say() { echo "[$stage] $*"; }
dir() { mkdir -p "$OUT/$1"; echo "$OUT/$1"; }
cmdlog() { echo "\$ $*" >> "$OUT/$stage/cmd.txt"; }
sh_robot() { cmdlog "adb shell $*"; $A shell "$@"; }
need_robot() { $A get-state >/dev/null 2>&1 || { echo "no adb at $HOST; enable root adb from the app's Lab first (pierluigizagaria/robomaster-s1-hacks)" >&2; exit 1; }; }
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
    say "static capture done; now the strace. USE THE APP: camera, a sound, hit each armour plate,"
    say "change the LED colour and effect, press the controller button, person recognition. 70 s."
    sh_robot "top -n 1 -m 15" > "$D/top_before.txt" 2>&1 || true
    # Bounded: `strace -p` only ends with the traced process, so each gets a 70 s timeout and `wait`
    # returns; the top samples cover the same window.
    sh_robot "$BB timeout 70 strace -f -tt -e trace=ioctl,write,read,open,openat -s 64 -o /data/hdvt.strace -p \$($BB pidof dji_hdvt_uav) & $BB timeout 70 strace -f -tt -e trace=ioctl,write,read,open,openat -s 64 -o /data/camera.strace -p \$($BB pidof dji_camera) & for i in 1 2 3 4 5 6 7; do top -n 1 -m 10 >> /data/top_vision.txt; $BB sleep 10; done; wait" || true
    $A pull /data/hdvt.strace "$D/hdvt_uav.strace" >/dev/null; $A pull /data/camera.strace "$D/dji_camera.strace" >/dev/null; $A pull /data/top_vision.txt "$D/top_vision.txt" >/dev/null
    for f in "$D"/*.txt "$D"/*.strace; do art "$f"; done ;;

  backup)
    need_robot; D=$(dir backup)
    sh_robot "ls -l /dev/block/platform/*/by-name" | tee "$D/by-name.txt"
    sh_robot "df /data" | tee "$D/df.txt"
    say "dumping each by-name partition to /data/bk/ then pulling; sizes decide staging"
    sh_robot "mkdir -p /data/bk; for p in /dev/block/platform/*/by-name/*; do n=\$($BB basename \$p); $BB dd if=\$p of=/data/bk/\$n.img bs=1M 2>/dev/null && $BB sha256sum /data/bk/\$n.img; done" | tee "$D/SHA256SUMS.robot"
    $A pull /data/bk "$D/" >/dev/null
    ( cd "$D/bk" && shasum -a 256 *.img > ../SHA256SUMS )
    # Both sides reduced to `<sha> <name>`: busybox prints `/data/bk/<name>.img`, the Mac prints
    # `<name>.img` with two spaces, and comparing them raw never matched.
    norm() { awk '{ sub(".*/", "", $2); print $1, $2 }' "$1" | sort; }
    diff <(norm "$D/SHA256SUMS.robot") <(norm "$D/SHA256SUMS") && say "SHA robot == Mac" || say "SHA MISMATCH — see $D"
    sh_robot "rm -rf /data/bk"
    for f in "$D"/SHA256SUMS "$D"/bk/*.img; do art "$f"; done ;;

  exec)
    need_robot; D=$(dir exec); ROOTFS="${S1_ROOTFS:-$HOME/Development/RoboMaster-S1/out/rootfs/rootfs-bookworm-armhf.tar}"
    { sh_robot "getenforce; cp /system/xbin/busybox /data/bb && chmod 755 /data/bb && /data/bb true && echo EXEC_IN_DATA_OK || echo EXEC_IN_DATA_DENIED"; } | tee "$D/exec.txt"
    if grep -q DENIED "$D/exec.txt"; then say "execution in /data denied: the OWNER runs 'setenforce 0' (or we try the microSD); re-run this stage"; exit 2; fi
    [ -f "$ROOTFS" ] || { say "no rootfs at $ROOTFS; run scripts/s1/build-rootfs.sh"; exit 1; }
    say "pushing rootfs ($(du -h "$ROOTFS" | cut -f1)) and extracting into $CHROOT"
    $A push "$ROOTFS" /data/rootfs.tar >/dev/null
    { sh_robot "mkdir -p $CHROOT && cd $CHROOT && $BB tar -xf /data/rootfs.tar && rm /data/rootfs.tar && mount -o bind /dev $CHROOT/dev; mount -t proc proc $CHROOT/proc; mount -t sysfs sys $CHROOT/sys; chroot $CHROOT /bin/true && echo CHROOT_OK; chroot $CHROOT /bin/uname -a; chroot $CHROOT /usr/bin/ldd --version | head -1"; } | tee -a "$D/exec.txt"
    art "$D/exec.txt" ;;

  stop)
    need_robot; D=$(dir stop)
    sh_robot "stop dji_hdvt_uav; stop dji_monitor; setprop ctl.stop dji_hdvt_uav; setprop ctl.stop dji_monitor; kill \$($BB pidof dji_hdvt_uav) 2>/dev/null; $BB sleep 1; $BB pidof dji_hdvt_uav >/dev/null && echo STILL_ALIVE || echo hdvt_uav_stopped" | tee "$D/stop.txt"
    sh_robot "for p in \$($BB ls /proc | $BB grep -E '^[0-9]+\$'); do $BB ls -l /proc/\$p/fd 2>/dev/null | $BB grep -q ttyS3 && echo \"ttyS3 held by \$p \$($BB cat /proc/\$p/comm)\"; done; echo owners_listed" | tee "$D/owners.txt"
    art "$D/stop.txt"; art "$D/owners.txt" ;;

  drive)
    need_robot; D=$(dir drive); : "${S1_BAUD:?S1_BAUD is the baud the capture stage measured with stty}"
    say "starting s1d (baud $S1_BAUD) and netpadd in the chroot; WHEELS OFF THE GROUND"
    # `nohup`, so the daemons outlive the adb shell that started them.
    sh_robot "$BB nohup chroot $CHROOT /opt/robot/bin/s1d --port /dev/ttyS3 --baud $S1_BAUD ${S1_ARGS:-} > /data/s1d.log 2>&1 &"
    sleep 2
    sh_robot "$BB nohup chroot $CHROOT /opt/robot/bin/netpadd > /data/netpadd.log 2>&1 &"; sleep 1
    # `robotctl health` exits non-zero on an unhealthy verdict; that is a result to record, not a
    # reason to stop the stage before the logs are pulled.
    sh_robot "chroot $CHROOT /opt/robot/bin/robotctl health --json" > "$D/health.json" 2>&1 || say "robotctl health rc=$? — the verdict is in health.json"
    cat "$D/health.json"
    say "on the Mac now:  duckctl udp-pad ${HOST%%:*}    — Start arms the chassis; sticks drive; Ctrl-C when done"
    say "collecting robot.state for 60 s meanwhile"
    sh_robot "$BB timeout 60 chroot $CHROOT /opt/robot/bin/robotctl monitor --json" > "$D/monitor.log" 2>&1 || true
    $A pull /data/s1d.log "$D/s1d.log" >/dev/null; $A pull /data/netpadd.log "$D/netpadd.log" >/dev/null
    for f in "$D"/health.json "$D"/monitor.log "$D"/s1d.log "$D"/netpadd.log; do art "$f"; done ;;

  events)
    need_robot; D=$(dir events)
    for c in "on --color red" "on --color green" "on --color blue" "off"; do sh_robot "chroot $CHROOT /opt/robot/bin/robotctl flashlight $c"; sleep 1; done | tee "$D/led_effects.txt"
    say "now TAP each armour plate, point an IR source if you have one, and PRESS the button; 30 s"
    sh_robot "$BB timeout 30 $BB cat /dev/input/event0 | $BB xxd" > "$D/event0.hex" 2>&1 || true
    $A pull /data/s1d.log "$D/s1d.log" >/dev/null
    grep -E "armour hit|infrared hit|button" "$D/s1d.log" | tee "$D/events_in_log.txt" || true
    for f in "$D"/led_effects.txt "$D"/event0.hex "$D"/s1d.log; do art "$f"; done ;;

  camera)
    need_robot; D=$(dir camera)
    sh_robot "stop dji_camera; setprop ctl.stop dji_camera; $BB sleep 1; chroot $CHROOT /usr/bin/v4l2-ctl -d /dev/video0 --all" | tee "$D/v4l2_all.txt" || true
    sh_robot "chroot $CHROOT /usr/bin/v4l2-ctl -d /dev/video0 --list-formats-ext" | tee "$D/formats.txt" || true
    sh_robot "chroot $CHROOT /usr/bin/v4l2-ctl -d /dev/video0 --set-fmt-video=width=1280,height=720,pixelformat=NV12 --stream-mmap --stream-count=1 --stream-to=/data/frame.nv12 && ls -l /data/frame.nv12" | tee -a "$D/formats.txt" || true
    $A pull /data/frame.nv12 "$D/frame.nv12" >/dev/null || true
    [ -f "$D/frame.nv12" ] && ffmpeg -loglevel error -y -f rawvideo -pix_fmt nv12 -s 1280x720 -i "$D/frame.nv12" "$D/frame.png" && say "open $D/frame.png"
    for f in "$D"/formats.txt "$D"/frame.nv12 "$D"/frame.png; do [ -f "$f" ] && art "$f"; done ;;

  audio)
    need_robot; D=$(dir audio)
    sh_robot "chroot $CHROOT /usr/bin/amixer -c 0 contents" > "$D/amixer_contents.txt" 2>&1 || true
    say "route per the hdvt_uav strace (tinymix/amixer values) goes in $D/route.sh before this runs; PLAY MUSIC NOW"
    [ -f "$D/route.sh" ] && sh_robot "$(cat "$D/route.sh")"
    sh_robot "chroot $CHROOT /usr/bin/arecord -D hw:0,1 -f S16_LE -r 16000 -c 1 -d 60 /data/mic.wav 2>&1 | $BB tee /data/stream_60s.txt" || true
    $A pull /data/mic.wav "$D/mic.wav" >/dev/null || true; $A pull /data/stream_60s.txt "$D/stream_60s.txt" >/dev/null || true
    say "aplay while driving: run stage drive in another terminal, then this plays 20 s and samples top"
    sh_robot "chroot $CHROOT /usr/bin/aplay -D hw:0,0 /data/mic.wav & for i in 1 2 3 4; do top -n 1 -m 8; $BB sleep 5; done" > "$D/cpu_play_drive.txt" 2>&1 || true
    for f in "$D"/mic.wav "$D"/stream_60s.txt "$D"/cpu_play_drive.txt; do [ -f "$f" ] && art "$f"; done ;;

  app)
    D=$(dir app); say "with hdvt_uav/monitor stopped: open the app, note connect/video/control. Type notes, Ctrl-D:"; cat > "$D/obs.txt"; art "$D/obs.txt" ;;

  restore)
    need_robot; D=$(dir restore)
    sh_robot "reboot" || true; say "rebooting; re-enable root adb from the app's Lab, then press Enter"; read -r _
    need_robot; sh_robot "$BB pidof dji_hdvt_uav && echo hdvt_uav_alive; getprop | $BB grep init.svc.dji" | tee "$D/after.txt"; art "$D/after.txt" ;;

  frames)
    need_robot; D=$(dir frames)
    for geo in 1280,720 640,360; do w=${geo%,*}; h=${geo#*,}
        sh_robot "chroot $CHROOT /usr/bin/gst-launch-1.0 -v v4l2src device=/dev/video0 num-buffers=300 ! video/x-raw,width=$w,height=$h ! fpsdisplaysink video-sink=fakesink text-overlay=false signal-fps-measurements=true 2>&1 | $BB grep -E 'fps|rendered|dropped' | $BB tail -5 & for i in 1 2 3; do top -n 1 -m 5; $BB sleep 4; done; wait" > "$D/fps_${h}p.txt" 2>&1 || true
    done
    sh_robot "chroot $CHROOT /usr/bin/gst-launch-1.0 v4l2src device=/dev/video0 num-buffers=1 ! video/x-raw,width=640,height=360 ! pngenc ! filesink location=/data/sample.png" >/dev/null 2>&1 || true
    $A pull /data/sample.png "$D/sample.png" >/dev/null 2>&1 || true
    for f in "$D"/fps_720p.txt "$D"/fps_360p.txt "$D"/sample.png; do [ -f "$f" ] && art "$f"; done ;;

  infer)
    need_robot; D=$(dir infer); ORT="${S1_ORT:-$HOME/Development/RoboMaster-S1/out/onnxruntime}"; MODEL="$OUT/duck_detect.onnx"
    [ -f "$MODEL" ] || curl --fail --location --silent --show-error -o "$MODEL" "https://huggingface.co/pollen-robotics/microduck-duck-detector/resolve/duck-v1/duck_detect.onnx"
    FRAMES="${S1_FRAMES:-$OUT/frames}"   # stage 11's PNG lands there; any directory of jpg/png works
    ls "$FRAMES"/*.png "$FRAMES"/*.jpg >/dev/null 2>&1 || { say "no frames in $FRAMES — run stage 11 or set S1_FRAMES"; exit 1; }
    # `duck-bench` is the NPU path; the S1 has none. `s1-infer` is the same measurement over the
    # detector's ONNX path, cross-compiled here and built natively below for the other tier.
    if ls "$ORT"/libonnxruntime.so* >/dev/null 2>&1; then
        cargo s1 -p s1d --bin s1-infer >/dev/null
        $A push target/armv7-unknown-linux-gnueabihf/release/s1-infer "$CHROOT/opt/robot/bin/s1-infer" >/dev/null
        $A push "$ORT"/libonnxruntime.so* "$CHROOT/usr/lib/" >/dev/null; $A push "$MODEL" "$CHROOT/opt/robot/duck_detect.onnx" >/dev/null
        sh_robot "mkdir -p $CHROOT/opt/robot/frames"
        for f in "$FRAMES"/*.png "$FRAMES"/*.jpg; do [ -f "$f" ] && $A push "$f" "$CHROOT/opt/robot/frames/" >/dev/null; done
        sh_robot "ORT_DYLIB_PATH=/usr/lib/libonnxruntime.so.1.28.0 chroot $CHROOT /opt/robot/bin/s1-infer --model /opt/robot/duck_detect.onnx --frames /opt/robot/frames --hz 0" > "$D/onboard.txt" 2>&1 || true
    else
        cp "$ORT/build.log" "$D/ort_build_failed.txt" 2>/dev/null || echo "no libonnxruntime armv7 and no build log" > "$D/ort_build_failed.txt"
    fi
    MAC_ORT="${S1_ORT_MAC:-$HOME/Development/RoboMaster-S1/out/onnxruntime-mac/onnxruntime-osx-arm64-1.28.0/lib/libonnxruntime.1.28.0.dylib}"
    cargo build --release -p s1d --bin s1-infer >/dev/null
    ORT_DYLIB_PATH="$MAC_ORT" target/release/s1-infer --model "$MODEL" --frames "$FRAMES" --hz 0 > "$D/offboard.txt" 2>&1 || true
    for f in "$D"/onboard.txt "$D"/offboard.txt "$D"/ort_build_failed.txt; do [ -f "$f" ] && art "$f"; done ;;

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
