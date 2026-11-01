#!/usr/bin/env bash
# Build the S1's daemons for armv7 and put them in the robot's chroot — `dev-push.sh`'s job, by adb
# instead of ssh, into a chroot instead of a release directory.
#
# The build needs three things zigbuild does not supply, checked here as dev-push.sh checks
# cargo-zigbuild: zig (the cross C toolchain and glibc floor), cargo-zigbuild, and the target's
# Rust std (`rustup target add`), without which the build fails with "can't find crate for std".
#
# Usage: scripts/s1/s1-push.sh [--build-only] [host:port]     (default host: $S1_HOST or 192.168.1.236:5555)
set -euo pipefail

cd "$(dirname "$0")/../.."
HOST="${S1_HOST:-192.168.1.236:5555}"
CHROOT="${S1_CHROOT:-/data/debian}"
BUILD_ONLY=no
for a in "$@"; do
    case "$a" in
        --build-only) BUILD_ONLY=yes ;;
        *) HOST="$a" ;;
    esac
done

missing=0
command -v zig >/dev/null 2>&1 || { echo "zig is not installed:            brew install zig" >&2; missing=1; }
command -v cargo-zigbuild >/dev/null 2>&1 || { echo "cargo-zigbuild is not installed: cargo install cargo-zigbuild --locked" >&2; missing=1; }
rustup target list --installed 2>/dev/null | grep -q '^armv7-unknown-linux-gnueabihf$' || { echo "the armv7 std is missing:        rustup target add armv7-unknown-linux-gnueabihf" >&2; missing=1; }
[ "$missing" -eq 0 ] || exit 1

echo "[s1-push] cargo s1 -p s1d -p netpadd -p robotctl"
cargo s1 -p s1d -p netpadd -p robotctl
BIN=target/armv7-unknown-linux-gnueabihf/release
for b in s1d s1ctl s1-cam netpadd robotctl; do
    file "$BIN/$b" | grep -q "ELF 32-bit.*ARM" || { echo "$BIN/$b is not an ARM 32-bit ELF" >&2; exit 1; }
done
echo "[s1-push] built: $(ls -la $BIN/s1d $BIN/s1ctl $BIN/s1-cam $BIN/netpadd $BIN/robotctl | awk '{print $5, $9}' | tr '\n' ' ')"
[ "$BUILD_ONLY" = no ] || exit 0

A="adb -s $HOST"
$A shell "test -d $CHROOT/bin" || { echo "no chroot at $CHROOT on $HOST; run bench.sh exec first" >&2; exit 1; }
$A shell "mkdir -p $CHROOT/opt/robot/bin $CHROOT/etc/robot $CHROOT/run $CHROOT/var/lib/robot"
for b in s1d s1ctl s1-cam netpadd robotctl; do
    $A push "$BIN/$b" "$CHROOT/opt/robot/bin/$b" >/dev/null
done
$A push deploy/s1/robotd.toml "$CHROOT/etc/robot/robotd.toml" >/dev/null
# The microphone for the duck's 16 kHz mono consumers (`pet-detect`, the sound sentry): `micleft`, a plug
# over hw:0,0 that resamples and keeps the LEFT channel — the one the voice is on (measured 2026-10-31).
$A push deploy/s1/asound.conf "$CHROOT/etc/asound.conf" >/dev/null
# The mixer as the stock leaves it (57 controls, `alsactl store` during the discarded floor session),
# restored by `s1d --alsa-state` at start; a boot where the stock never ran has not been tried.
$A push deploy/s1/asound.state.stock "$CHROOT/opt/robot/asound.state.stock" >/dev/null
$A shell "chmod 755 $CHROOT/opt/robot/bin/*"
# A sound bank for `robot.sound`: the stock firmware's own wavs (`/system`, bind-mounted read-only
# into the chroot by `bench.sh exec`) under the duck's tags, until phase 2 gives the S1 a voice.
# `busybox ln`: the Android shell's own `ln` is a toolbox applet that is not on the adb shell's
# path, and the loop silently created nothing (measured 2026-10-30: an empty bank on the robot).
BB=/system/xbin/busybox
$A shell "mkdir -p $CHROOT/opt/robot/sounds && cd $CHROOT/opt/robot/sounds && for pair in chirp:media_sound_demo_recognize_success greet:power_on alarm:media_sound_demo_attacked inquire:media_sound_demo_scanning peck:sleeping coo:media_sound_demo_gimbal_rotate wheee:engine_start shoot:ir_gun_shoot; do $BB ln -sf /mnt/system/audio/sound/\${pair#*:}.wav \${pair%%:*}.wav; done; $BB ls -l"
n=$($A shell "$BB ls $CHROOT/opt/robot/sounds | $BB wc -l" | tr -d '\r ')
[ "$n" = "8" ] || { echo "the sound bank has $n entries, expected 8" >&2; exit 1; }
# The chroot keeps Android's PATH unless told otherwise; `aplay`/`amixer` are found only with Debian's.
ENV="$BB chroot $CHROOT /usr/bin/env PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
echo "[s1-push] installed into $HOST:$CHROOT/opt/robot/bin (sound bank: $n files)"
echo "[s1-push] run (stock stopped, baud from bench.sh capture):"
echo "  adb -s $HOST shell $ENV /opt/robot/bin/s1d --port /dev/ttyS3 --baud <BAUD> --sounds /opt/robot/sounds"
echo "  adb -s $HOST shell $ENV /opt/robot/bin/netpadd"
echo "  adb -s $HOST shell $ENV /opt/robot/bin/robotctl health"
echo "  adb -s $HOST shell $ENV /opt/robot/bin/s1ctl version"
