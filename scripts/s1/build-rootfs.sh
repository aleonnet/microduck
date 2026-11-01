#!/usr/bin/env bash
# Build the Debian armhf chroot the S1's daemons run in, as a tarball, from the official image.
#
# Why a chroot: the S1's controller is Android 4.4 on kernel 3.10 with a bionic userland; our
# daemons are glibc binaries. Kernel 3.10 is above the floor of both the Rust target
# (armv7-unknown-linux-gnueabihf: kernel 3.2+, glibc 2.17) and Debian bookworm's glibc 2.36, so a
# Debian userland runs on it unmodified, extracted under /data (or the microSD) and entered with
# `chroot`. No flashing, nothing written to /system: a reboot leaves the stock firmware untouched.
#
# Why docker export rather than debootstrap: the Mac has docker and no debootstrap, the image is
# the same bookworm-slim Debian publishes for arm32v7, and `apt-get` runs inside it with the
# robot's architecture, so the packages are exactly what `apt` would install on the robot.
#
# What goes in, and why each: alsa-utils (arecord/aplay — what robotd's voice and pet-detect use),
# v4l-utils and gstreamer (v4l2-ctl lists formats; its stream rebooted the robot, v4l2src is off too),
# strace (the tool every bring-up answer came from), procps (top for the CPU figures).
#
# Usage: scripts/s1/build-rootfs.sh [out-dir]   → <out-dir>/rootfs-bookworm-armhf.tar + .sha256 + .packages
set -euo pipefail

OUT="${1:-$HOME/Development/RoboMaster-S1/out/rootfs}"
IMAGE="debian:bookworm-slim"
PLATFORM="linux/arm/v7"
PACKAGES="alsa-utils v4l-utils gstreamer1.0-tools gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-alsa strace procps ca-certificates"
NAME="s1-rootfs-build-$$"

if ! docker version >/dev/null 2>&1; then
    echo "docker is not running; open -a Docker" >&2
    exit 1
fi
mkdir -p "$OUT"

echo "[rootfs] installing into $IMAGE ($PLATFORM): $PACKAGES"
docker run --platform "$PLATFORM" --name "$NAME" "$IMAGE" sh -c "
    set -e
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $PACKAGES >/dev/null
    apt-get clean
    rm -rf /var/lib/apt/lists/*
    mkdir -p /run /opt/robot/bin /etc/robot /var/lib/robot
    uname -m; dpkg --print-architecture; ldd --version | head -1
    dpkg-query -W -f='\${Package} \${Version}\n' > /root/packages.txt
"
docker cp "$NAME:/root/packages.txt" "$OUT/rootfs-bookworm-armhf.packages"
docker export "$NAME" > "$OUT/rootfs-bookworm-armhf.tar"
docker rm "$NAME" >/dev/null
( cd "$OUT" && shasum -a 256 rootfs-bookworm-armhf.tar > rootfs-bookworm-armhf.tar.sha256 )
echo "[rootfs] $(du -h "$OUT/rootfs-bookworm-armhf.tar" | cut -f1) → $OUT/rootfs-bookworm-armhf.tar"
cat "$OUT/rootfs-bookworm-armhf.tar.sha256"
echo "[rootfs] packages: $(wc -l < "$OUT/rootfs-bookworm-armhf.packages") (list beside the tar)"
